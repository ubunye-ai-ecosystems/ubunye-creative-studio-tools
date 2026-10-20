//! kopanod engine: one device identity, many projects, live sync both ways (09, slice S1).
//!
//! Why one service per computer: one device key must never run in two programs (E37c: two helpers
//! with one id answered each other's requests; E39: the relay refused a second endpoint with the
//! same key). Every app (Light, Photo, Film) talks to this service instead of opening its own.
//!
//! Home folder:
//!   device.key, cert.json          this device (key in the OS key store when protected)
//!   api.token                      apps must show it on the local API
//!   projects/<id8>/                kopano.redb, project, head, app.json, peers.json, media.json
//!   media/                         photos' files: blobs, previews, originals (media.rs)
//!
//! Sync protocol `kopano/sync/1` = the lab's `kopano/sync/0` plus one frame after the certificate
//! naming the project, so one endpoint serves every project. Live edits travel as compact pushes
//! (E20/E24): each side WATCHES each teammate device it knows and is watched by them, so an edit
//! reaches the other side in about half a round trip with no pull command.

#[path = "../../kopano-lab/src/keystore.rs"]
pub mod keystore;
pub mod approve;
pub mod film_binding;
pub mod media;
pub mod photo_binding;
pub mod unlinked;

use anyhow::{anyhow, bail, Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use kopano_core::identity::{now_ms, DeviceCert, DeviceKey};
use kopano_core::project::{self, Item, Project};
use kopano_core::repo::{Integrated, Repo};
use kopano_core::store::{Id, ObjectStore, RedbStore};
use kopano_core::tree;
use kopano_core::connector::Connector;
use kopano_lab::light::LightConnector;
use kopano_lab::ToyEditor;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex};

pub const ALPN: &[u8] = b"kopano/sync/1";
const MAX_FRAME: usize = 64 << 20;

// ---- device and projects -----------------------------------------------------------------------

pub struct Device {
    pub home: PathBuf,
    pub key: DeviceKey,
    pub cert: DeviceCert,
}

impl Device {
    pub fn open(home: &Path) -> Result<Self> {
        let key = DeviceKey::from_bytes(keystore::load(home)?);
        let cert: DeviceCert = serde_json::from_slice(&std::fs::read(home.join("cert.json")).context("no cert.json: link this device first")?)?;
        if cert.body.device != key.public_hex() {
            bail!("cert.json is for another device key");
        }
        Ok(Self { home: home.into(), key, cert })
    }
    pub fn id(&self) -> String {
        self.cert.body.device.clone()
    }
}

/// One project on this device. Reads are free; anything that moves the head goes through `write`.
pub struct Proj {
    pub dir: PathBuf,
    pub store: RedbStore,
    pub project: Project,
    write: Mutex<()>,
    heads: broadcast::Sender<Id>,
    pushed: std::sync::Mutex<HashMap<(Vec<Id>, Id), Arc<(u64, usize, Vec<u8>)>>>,
    following: std::sync::Mutex<HashSet<String>>,
    /// per followed device: wakes its watch loop (a teammate that connects to us again may have
    /// restarted, and our own watch of it may hang on a dead connection until QUIC times out)
    kicks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// per device that watches us: the `boot` its last watch carried
    boots: std::sync::Mutex<HashMap<String, u64>>,
    online: std::sync::Mutex<BTreeMap<String, String>>,
    binding: std::sync::Mutex<Binding>,
    light: std::sync::Mutex<Option<LightConnector>>,
    /// the Film window, when this project lives in Film (film_binding.rs)
    film: std::sync::Mutex<Option<film_binding::Slot>>,
    /// the node's media (set when the node starts): where preview files live
    media: std::sync::OnceLock<Arc<media::Media>>,
    /// a Photo window bound to this project (photo_binding.rs)
    photo: photo_binding::Slot,
    /// "I approve changes" projects: refs, members log, review (approve.rs)
    pub review: approve::State,
}

/// Which app holds this project's items on this device.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Binding {
    /// the lab's toy editor in app.json (tests without a real app)
    Toy,
    /// the running Light desktop app (`lightcraft --control PORT`)
    LightControl { port: u16 },
    /// a headless Light engine with the demo library (automated tests)
    LightHeadless { cli: String },
    /// the running Ubunye Photo app (`photocraft --control PORT --control-token-file FILE`);
    /// tiles live in `media_dir` (default: the project's `photo-media` folder). photo_binding.rs
    PhotoControl {
        port: u16,
        token_file: String,
        #[serde(default)]
        media_dir: Option<String>,
    },
    /// a headless Photo engine with the demo document (automated tests)
    PhotoHeadless {
        cli: String,
        #[serde(default)]
        media_dir: Option<String>,
    },
    /// the running Film desktop app (`filmcraft --control PORT`), see film_binding.rs
    FilmControl { port: u16 },
    /// a headless Film engine (`filmcraft-cli mcp`) on a project file, or Film's demo (tests)
    FilmHeadless { cli: String, project: Option<String> },
}

impl Proj {
    fn open(dir: &Path) -> Result<Self> {
        let store = RedbStore::open(dir.join("kopano.redb"))?;
        let pid = Id::parse(std::fs::read_to_string(dir.join("project"))?.trim())?;
        let project = Project::load(&store, &pid)?;
        Ok(Self {
            dir: dir.into(),
            store,
            project,
            write: Mutex::new(()),
            heads: broadcast::channel(64).0,
            pushed: Default::default(),
            following: Default::default(),
            kicks: Default::default(),
            boots: Default::default(),
            online: Default::default(),
            binding: std::sync::Mutex::new(std::fs::read(dir.join("binding.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Binding::Toy)),
            light: Default::default(),
            film: Default::default(),
            media: Default::default(),
            photo: Default::default(),
            review: approve::State::load(dir),
        })
    }
    /// Run `f` against this project's app: the toy editor file, or Light (connected on first use
    /// and again after a dropped connection).
    fn with_app<R>(&self, dev: &Device, f: impl FnOnce(&mut dyn Connector) -> Result<R>) -> Result<R> {
        match self.binding() {
            Binding::Toy => tokio::task::block_in_place(|| {
                let mut app = self.app()?;
                let r = f(&mut app)?;
                self.save_app(&app)?;
                Ok(r)
            }),
            b if b.is_photo() => photo_binding::with_photo(self, dev, f),
            Binding::FilmControl { .. } | Binding::FilmHeadless { .. } => film_binding::with_app(self, dev, f),
            // media.rs: photos whose preview is still on its way count as the project's state
            _ => self.with_light(dev, |c| f(&mut media::View::new(&self.store, self.head(), c))),
        }
    }
    /// Run `f` against this project's Light window itself (connected on first use and again
    /// after a dropped connection; a window that connects is brought to the project first).
    fn with_light<R>(&self, dev: &Device, f: impl FnOnce(&mut LightConnector) -> Result<R>) -> Result<R> {
        let binding = self.binding();
        tokio::task::block_in_place(|| {
            let mut g = self.light.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_none() {
                let mut c = match &binding {
                    Binding::LightControl { port } => LightConnector::live(*port).context("Light is not running with its control port")?,
                    Binding::LightHeadless { cli } => LightConnector::headless(cli)?,
                    _ => unreachable_toy()?,
                };
                media::prepare(self.media.get(), self, &mut c);
                self.reconcile(dev, &mut media::View::new(&self.store, self.head(), &mut c))?;
                *g = Some(c);
            }
            let r = f(g.as_mut().context("no app")?);
            if r.is_err() {
                *g = None; // reconnect next time
            }
            r
        })
    }
    /// A window that (re)connects is brought to the project first; it never speaks first.
    ///
    /// S3b found the data loss this prevents: Light restarted as a fresh window, kopanod was
    /// attached to it, and the next save read the fresh window's untouched photos as "the user
    /// reset everything" and pushed that to the team, wiping everyone's edits. Whatever the window
    /// held is not thrown away either: it is kept as its own version (in `kept.json`, off the
    /// main line) so it can be restored by hand, and the project's head is applied into the app.
    ///
    /// S8 found the opposite loss: a window that stayed open while kopanod was stopped (or that
    /// was closed and opened again on its own library meanwhile) holds the person's edits made in
    /// that time. Those are states the project never had; putting the head back over them threw
    /// the work away (only `kept.json` had it). So only items whose window state is one the
    /// project held before (an older value: a fresh window, an old file) are brought to the head;
    /// a state the project never had is the person's work, stays in the window, and the next save
    /// takes it like any edit (main for the owner, the member's branch in approve mode).
    fn reconcile(&self, dev: &Device, app: &mut dyn Connector) -> Result<()> {
        let Some(h) = self.head() else { return Ok(()) };
        let root = project::load_version(&self.store, &h)?.tree()?;
        let want = tree::entries(&self.store, root)?;
        let have = app.snapshot()?;
        let mut differ = Vec::new();
        for (k, item) in &have {
            let id = item.put(&self.store)?;
            if want.get(k).is_some_and(|w| *w != id) {
                differ.push((k.clone(), id));
            }
        }
        if differ.is_empty() {
            return Ok(());
        }
        // every version this device knows: in approve mode the members' branches too, so the
        // owner's window that still shows a member's version (she was looking at it when the
        // window or the service went down) is put back, never saved into main as her own edit
        let mut heads = vec![h];
        if approve::on(self) {
            heads.extend(approve::ref_versions(self));
        }
        let seen = seen_states(&self.store, &heads, differ.iter().map(|(k, _)| k))?;
        let (changes, work): (Vec<_>, Vec<_>) = differ.into_iter().partition(|(k, id)| seen.get(k).is_some_and(|s| s.contains(id)));
        let work: BTreeSet<String> = work.into_iter().map(|(k, _)| k).collect();
        if !work.is_empty() {
            eprintln!("[kopanod] {}: a window was linked again with {} item(s) edited while it was not linked; kept in the window as edits", self.project.id.short(), work.len());
        }
        let changes: Vec<(String, Option<Id>)> = changes.into_iter().map(|(k, id)| (k, Some(id))).collect();
        if changes.is_empty() {
            return Ok(());
        }
        let n = changes.len();
        let kept_root = tree::update(&self.store, root, changes)?;
        let kept = project::commit(&self.store, &self.project, vec![h], kept_root, "Kept: this window's own state when it was linked again", &dev.cert, &dev.key)?;
        self.store.flush()?;
        let mut list: Vec<String> = std::fs::read(self.dir.join("kept.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        list.push(kept.hex());
        kopano_core::store::write_atomic(&self.dir.join("kept.json"), &serde_json::to_vec(&list)?)?;
        let info = app.info();
        for (k, id) in &want {
            if work.contains(k) {
                continue;
            }
            let item = Item::get(&self.store, id)?;
            if info.owns(&item.kind) && have.get(k) != Some(&item) {
                app.apply(k, Some(&item))?;
            }
        }
        eprintln!("[kopanod] {}: a window was linked again; brought it to the project ({n} item(s) held older states, kept as {})", self.project.id.short(), kept.short());
        Ok(())
    }
    pub fn binding(&self) -> Binding {
        self.binding.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Point this project at the app again, e.g. Light restarted on another control port.
    pub fn rebind(&self, b: Binding) -> Result<()> {
        std::fs::write(self.dir.join("binding.json"), serde_json::to_vec(&b)?)?;
        *self.binding.lock().unwrap_or_else(|e| e.into_inner()) = b;
        *self.light.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.photo.reset();
        *self.film.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
    /// How many photos (items before the `#lane`) differ between two versions.
    pub fn photos_changed(&self, before: Option<Id>, after: Option<Id>) -> usize {
        let tree_of = |v: Option<Id>| v.and_then(|v| project::load_version(&self.store, &v).ok()).and_then(|v| v.tree().ok().flatten());
        let (a, b) = match (tree::entries(&self.store, tree_of(before)), tree::entries(&self.store, tree_of(after))) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return 0,
        };
        let base = |k: &String| k.rsplit_once('#').map(|(x, _)| x.to_string()).unwrap_or_else(|| k.clone());
        let mut changed = BTreeSet::new();
        for (k, id) in &b {
            if a.get(k) != Some(id) && !k.starts_with("conflict") {
                changed.insert(base(k));
            }
        }
        for k in a.keys() {
            if !b.contains_key(k) {
                changed.insert(base(k));
            }
        }
        changed.len()
    }
    pub fn head(&self) -> Option<Id> {
        std::fs::read_to_string(self.dir.join("head")).ok().and_then(|s| Id::parse(s.trim()).ok())
    }
    /// Objects first, pointer second: the head file may only name a version whose objects are
    /// already durable. (S1 found a service stopped between the two pointing at objects it never
    /// kept, then refusing every sync with "missing object".)
    fn set_head(&self, id: Option<Id>) -> Result<()> {
        match id {
            Some(h) => {
                self.store.flush()?;
                kopano_core::store::write_atomic(&self.dir.join("head"), h.hex().as_bytes())
            }
            None => Ok(()),
        }
    }
    /// A head left by an older build (or a disk that lied about flushing) that names objects this
    /// store does not have: forget it and take the project from teammates again, instead of being
    /// stuck. Only edits that were never durable are lost.
    fn heal_head(&self) -> Option<String> {
        let h = self.head()?;
        let whole = project::history(&self.store, &[h]).and_then(|hist| {
            for (_, v) in &hist {
                tree::entries(&self.store, v.tree()?)?;
            }
            Ok(())
        });
        let e = whole.err()?;
        let _ = std::fs::rename(self.dir.join("head"), self.dir.join("head.broken"));
        Some(format!("{e:#}"))
    }
    pub fn app(&self) -> Result<ToyEditor> {
        let photos: BTreeMap<String, Item> = serde_json::from_slice(&std::fs::read(self.dir.join("app.json"))?)?;
        Ok(ToyEditor { photos })
    }
    fn save_app(&self, app: &ToyEditor) -> Result<()> {
        kopano_core::store::write_atomic(&self.dir.join("app.json"), &serde_json::to_vec_pretty(&app.photos)?)
    }
    fn repo<'a>(&'a self, dev: &Device) -> Result<Repo<'a>> {
        Ok(Repo { store: &self.store, project: Project::load(&self.store, &self.project.id)?, head: self.head(), cert: dev.cert.clone(), key: DeviceKey::from_bytes(dev.key.to_bytes()) })
    }
    fn check_peer(&self, cert: &DeviceCert, remote_id: &[u8; 32]) -> Result<()> {
        cert.check(&self.project.genesis.providers, now_ms())?;
        if cert.body.device != hex::encode(remote_id) {
            bail!("certificate is for another device than the one connecting");
        }
        if !self.is_member_device(cert) {
            bail!("{} is not a member of this project", cert.body.name);
        }
        Ok(())
    }
    /// On the project now: the genesis members, plus (approve mode) the signed members log, a
    /// blocked device refused.
    pub fn is_member_device(&self, cert: &DeviceCert) -> bool {
        if self.project.approve_mode() {
            return approve::members(self).is_ok_and(|m| m.device_ok(cert));
        }
        self.project.genesis.members.contains_key(&cert.body.account)
    }
    pub fn is_member_account(&self, account: &str) -> bool {
        if self.project.approve_mode() {
            return approve::members(self).is_ok_and(|m| m.is_member(account));
        }
        self.project.genesis.members.contains_key(account)
    }
    pub fn peers(&self) -> BTreeSet<String> {
        std::fs::read(self.dir.join("peers.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    fn add_peer(&self, device: &str) -> Result<bool> {
        let mut p = self.peers();
        if !p.insert(device.to_string()) {
            return Ok(false);
        }
        kopano_core::store::write_atomic(&self.dir.join("peers.json"), &serde_json::to_vec(&p)?)?;
        Ok(true)
    }
    /// device -> {name, account}, kept so an offline teammate still shows by name
    pub fn names(&self) -> BTreeMap<String, Value> {
        std::fs::read(self.dir.join("names.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    fn remember(&self, cert: &DeviceCert) {
        let mut n = self.names();
        let v = json!({"name": cert.body.name, "account": cert.body.account, "expires_ms": cert.body.expires_ms});
        if n.get(&cert.body.device) != Some(&v) {
            n.insert(cert.body.device.clone(), v);
            if let Ok(b) = serde_json::to_vec(&n) {
                let _ = kopano_core::store::write_atomic(&self.dir.join("names.json"), &b);
            }
        }
    }
    pub fn online(&self) -> BTreeMap<String, String> {
        self.online.lock().unwrap().clone()
    }
}

// ---- the node ----------------------------------------------------------------------------------

pub struct Node {
    pub device: Device,
    pub projects: std::sync::RwLock<HashMap<Id, Arc<Proj>>>,
    pub events: broadcast::Sender<Value>,
    endpoint: std::sync::OnceLock<Endpoint>,
    /// photos' files (media.rs), opened when the node starts
    media: std::sync::OnceLock<Arc<media::Media>>,
    /// a new random number each time the service starts: a watch carries it, so a teammate can
    /// tell that this service restarted (and that its own watch of us is on a dead connection)
    boot: u64,
}

impl Node {
    pub fn open(home: &Path) -> Result<Arc<Self>> {
        let device = Device::open(home)?;
        let mut projects = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(home.join("projects")) {
            for e in rd.flatten() {
                match Proj::open(&e.path()) {
                    Ok(p) => {
                        if let Some(e) = p.heal_head() {
                            eprintln!("[kopanod] {}: saved state was incomplete ({e}); taking it from teammates again", p.project.id.short());
                        }
                        projects.insert(p.project.id, Arc::new(p));
                    }
                    Err(err) => eprintln!("[kopanod] skipped {}: {err:#}", e.path().display()),
                }
            }
        }
        Ok(Arc::new(Self { device, projects: std::sync::RwLock::new(projects), events: broadcast::channel(1024).0, endpoint: Default::default(), media: Default::default(), boot: rand::random() }))
    }

    pub fn project(&self, id: &str) -> Result<Arc<Proj>> {
        let ps = self.projects.read().unwrap();
        ps.values().find(|p| p.project.id.hex() == id || p.project.id.hex().starts_with(id) && id.len() >= 8).cloned().context("no such project on this device")
    }

    fn emit(&self, v: Value) {
        eprintln!("[kopanod] {v}");
        let _ = self.events.send(v);
    }

    /// Bind the one endpoint and start following every known teammate device.
    pub async fn start(self: &Arc<Self>) -> Result<Router> {
        let sk = SecretKey::from_bytes(&self.device.key.to_bytes());
        let mut b = Endpoint::builder(presets::N0).secret_key(sk).alpns(vec![ALPN.to_vec(), iroh_blobs::ALPN.to_vec()]);
        if let Some(t) = transport() {
            b = b.transport_config(t);
        }
        if let Ok(u) = std::env::var("KOPANO_RELAY_URL") {
            let url: iroh::RelayUrl = u.parse().context("KOPANO_RELAY_URL")?;
            b = b.relay_mode(iroh::endpoint::RelayMode::Custom(iroh::RelayMap::from(url)));
        }
        let ep = b.bind().await?;
        let m = media::Media::open(&self.device.home).await?;
        let _ = self.media.set(m.clone());
        let blobs = iroh_blobs::BlobsProtocol::new(&m.store, Some(media::gate(self.clone())));
        let router = Router::builder(ep.clone()).accept(ALPN, Hub { node: self.clone() }).accept(iroh_blobs::ALPN, blobs).spawn();
        let _ = tokio::time::timeout(Duration::from_secs(10), ep.online()).await;
        let _ = self.endpoint.set(ep);
        let ps: Vec<Arc<Proj>> = self.projects.read().unwrap().values().cloned().collect();
        for p in ps {
            let _ = p.media.set(m.clone());
            for d in p.peers() {
                self.follow(p.clone(), d);
            }
            self.watch_app(p.clone());
        }
        Ok(router)
    }

    /// Keep watching one teammate device for one project, for as long as the service runs.
    /// E29: a closed lid or lost signal ends the connection; retry 1, 2, 4 ... 30 s apart, and each
    /// new watch says what we already have, so everything missed arrives as one compact push.
    pub fn follow(self: &Arc<Self>, p: Arc<Proj>, device: String) {
        if device == self.device.id() || !p.following.lock().unwrap().insert(device.clone()) {
            return;
        }
        let node = self.clone();
        let kick = Arc::new(tokio::sync::Notify::new());
        p.kicks.lock().unwrap().insert(device.clone(), kick.clone());
        tokio::spawn(async move {
            let mut wait = 1u64;
            loop {
                let started = Instant::now();
                let r = tokio::select! {
                    r = node.watch(&p, &device) => r,
                    _ = kick.notified() => Err(anyhow!("the teammate's service restarted; watching it anew")),
                };
                if let Err(e) = r {
                    if p.online.lock().unwrap().remove(&device).is_some() {
                        node.emit(json!({"event": "teammate", "project": p.project.id.hex(), "device": device, "online": false}));
                    }
                    eprintln!("[kopanod] {} on {}: {e:#}; retrying in {wait} s", &device[..8], p.project.id.short());
                }
                if started.elapsed().as_secs() > 60 {
                    wait = 1;
                }
                // S8: a teammate whose service restarted connects to us at once; waiting out the
                // back-off (or QUIC's idle timeout on the dead connection) kept its edits away 45 s
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(wait)) => wait = (wait * 2).min(30),
                    _ = kick.notified() => wait = 1,
                }
            }
        });
    }

    async fn watch(&self, p: &Arc<Proj>, device: &str) -> Result<()> {
        let ep = self.endpoint.get().context("not started")?.clone();
        let id = iroh::EndpointId::from_bytes(&hex::decode(device)?.try_into().map_err(|_| anyhow!("bad device id"))?)?;
        // address by id only: the n0 preset looks it up (pkarr), no address files
        let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from(id), ALPN)).await.map_err(|_| anyhow!("not reachable"))??;
        let (mut s, mut r) = conn.open_bi().await?;
        write_frame(&mut s, &serde_json::to_vec(&self.device.cert)?).await?;
        write_frame(&mut s, p.project.id.hex().as_bytes()).await?;
        let first = read_frame(&mut r).await?;
        if first.starts_with(b"refused") {
            bail!("{}", String::from_utf8_lossy(&first));
        }
        let their: DeviceCert = serde_json::from_slice(&first)?;
        p.check_peer(&their, conn.remote_id().as_bytes())?;
        p.online.lock().unwrap().insert(device.to_string(), their.body.name.clone());
        p.remember(&their);
        self.emit(json!({"event": "teammate", "project": p.project.id.hex(), "device": device, "name": their.body.name, "online": true}));
        let mut req = if approve::on(p) {
            approve::watch_request(p)
        } else {
            let have: Vec<String> = p.head().into_iter().map(|h| h.hex()).collect();
            json!({"op": "watch", "have": have})
        };
        req["boot"] = json!(self.boot);
        write_frame(&mut s, &serde_json::to_vec(&req)?).await?;
        loop {
            let note: Value = serde_json::from_slice(&read_frame(&mut r).await?)?;
            let z = read_frame(&mut r).await?;
            if approve::on(p) {
                let t = Instant::now();
                if let Some((what, conflicts, photos)) = approve::take(self, p, &note, &z).await? {
                    self.emit(json!({"event": "synced", "project": p.project.id.hex(), "from": their.body.name, "head": p.head().map(|h| h.short()),
                        "how": what, "clashes": conflicts, "photos": photos, "bytes": z.len(), "apply_ms": t.elapsed().as_millis() as u64,
                        "sent_ms": note.get("ms").and_then(|m| m.as_u64()), "now_ms": now_ms()}));
                }
                continue;
            }
            let Some(h) = note.get("head").and_then(|h| h.as_str()).and_then(|h| Id::parse(h).ok()) else { continue };
            let t = Instant::now();
            let raw = note.get("raw").and_then(|n| n.as_u64()).unwrap_or(0);
            let outcome = {
                let _w = p.write.lock().await;
                let before = p.head();
                let versions = kopano_core::wire::decode_compact(&p.store, &z, raw)?.versions;
                for v in &versions {
                    project::verify_version(&p.store, &p.project, v)?;
                }
                self.integrate(p, h)?.map(|(what, c)| (what, c, p.photos_changed(before, p.head())))
            };
            if let Some((what, conflicts, photos)) = outcome {
                if let Some(nh) = p.head() {
                    let _ = p.heads.send(nh);
                }
                self.emit(json!({"event": "synced", "project": p.project.id.hex(), "from": their.body.name, "head": p.head().map(|h| h.short()),
                    "how": what, "clashes": conflicts, "photos": photos, "bytes": z.len(), "apply_ms": t.elapsed().as_millis() as u64,
                    "sent_ms": note.get("ms").and_then(|m| m.as_u64()), "now_ms": now_ms()}));
            }
        }
    }

    /// Bring a verified head in. Returns what changed, or None if nothing did.
    fn integrate(&self, p: &Proj, theirs: Id) -> Result<Option<(&'static str, usize)>> {
        let mine = p.head();
        // Two devices that merged the same pair at the same moment hold two merge versions with
        // the SAME tree. Merging those again would make a third, and the next round a fourth:
        // they would never settle. Equal trees: both keep the smaller id, no new version.
        if let Some(m) = mine {
            if m != theirs && !project::is_ancestor(&p.store, &theirs, &m)? && !project::is_ancestor(&p.store, &m, &theirs)? {
                let (tm, tt) = (project::load_version(&p.store, &m)?.tree()?, project::load_version(&p.store, &theirs)?.tree()?);
                if tm == tt {
                    // Rank = (versions behind it, then the smaller id): a fast-forward always
                    // raises it, so settling must too. Settling on the smaller id alone let two
                    // forks whose trees cross-match (S6, after two windows were linked again)
                    // cycle: settle back to an older version, fast-forward, settle, for ever,
                    // with the window flipping between two states every second.
                    let rank = |v: &Id| project::history(&p.store, &[*v]).map(|h| (h.len(), std::cmp::Reverse(*v)));
                    if rank(&theirs)? > rank(&m)? {
                        p.set_head(Some(theirs))?;
                        return Ok(Some(("same content, settled", 0)));
                    }
                    return Ok(None);
                }
            }
        }
        // Our own head coming back (a teammate fast-forwarded to it and pushes it on), or one we
        // already contain: nothing to bring in, so the app is not touched. Going through
        // Repo::integrate here would first save the app's newest edit into the head without
        // telling anyone (S6 found a slider change that then waited ~30 s for the next edit).
        if let Some(m) = mine {
            if m == theirs || project::is_ancestor(&p.store, &theirs, &m)? {
                return Ok(None);
            }
        }
        // Photo refuses an update when the user edited the document after the snapshot the
        // update was built on (Repo::integrate builds it on the snapshot it saved, never on a
        // second read): try again, and the next try's first step saves that edit
        // (photo_binding.rs)
        let mut tries = 0;
        let (repo_head, r) = loop {
            let mut repo = p.repo(&self.device)?;
            match p.with_app(&self.device, |app| repo.integrate(app, &theirs)) {
                Err(e) if tries < 8 && kopano_lab::photo::is_refused(&e) => {
                    tries += 1;
                    eprintln!("[kopanod] {}: the app was edited while changes came in; trying again ({tries})", p.project.id.short());
                    tokio::task::block_in_place(|| std::thread::sleep(Duration::from_millis(100 * tries)));
                }
                r => break (repo.head, r?),
            }
        };
        p.set_head(repo_head)?;
        p.store.flush()?;
        Ok(match r {
            // the save before bringing in moved the head: it must still be pushed
            Integrated::AlreadyHad if repo_head != mine => Some(("saved an edit made meanwhile", 0)),
            Integrated::AlreadyHad => None,
            Integrated::FastForward { .. } => Some(("fast-forward", 0)),
            Integrated::Merged { conflicts, .. } => {
                if !conflicts.is_empty() {
                    eprintln!("[kopanod] {}: clashes kept for review: {}", p.project.id.short(), conflicts.iter().take(6).map(|c| format!("{} [{}]", c.key, c.fields.join(", "))).collect::<Vec<_>>().join("; "));
                }
                Some(("merged", conflicts.len()))
            }
        })
    }

    /// A local edit from an app: save a version and push it to every watcher.
    pub async fn set(&self, p: &Proj, key: &str, field: &str, value: Value) -> Result<Option<Id>> {
        let v = {
            let _w = p.write.lock().await;
            if p.binding() != Binding::Toy {
                bail!("this project lives in Light: edit it there");
            }
            let mut app = p.app()?;
            let item = app.photos.get_mut(key).context("no such item")?;
            let ptr = format!("/{}", field.replace('.', "/"));
            *item.state.pointer_mut(&ptr).context("no such setting")? = value;
            let mut repo = p.repo(&self.device)?;
            let v = repo.save(&mut app, "edit")?;
            p.save_app(&app)?;
            p.set_head(repo.head)?;
            approve::sign_mine(self, p)?;
            v
        };
        approve::notify(self, p);
        if let Some(h) = p.head() {
            let _ = p.heads.send(h);
        }
        self.emit(json!({"event": "saved", "project": p.project.id.hex(), "head": p.head().map(|h| h.short()), "now_ms": now_ms()}));
        Ok(v)
    }

    /// S3: notice edits made in the app itself. Every second, ask the app for its items and save
    /// a version only if something changed (Repo::save compares with the head), then push it.
    /// Light has no change feed yet (05 phase 1), so this is a poll; 1 s keeps "live" honest.
    pub fn watch_app(self: &Arc<Self>, p: Arc<Proj>) {
        if p.binding() == Binding::Toy {
            return;
        }
        if let Some(m) = self.media.get() {
            let _ = p.media.set(m.clone());
        }
        if p.binding().is_photo() {
            photo_binding::start(self, &p);
        } else {
            media::spawn(self.clone(), p.clone());
        }
        let node = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                // a device that just joined must take the project's versions first: saving its
                // own app state before that would start a second history and force a merge
                if p.head().is_none() {
                    continue;
                }
                // the owner is looking at a member's version in this window: nothing to save
                if p.review.viewing().is_some() {
                    continue;
                }
                let r = {
                    let _w = p.write.lock().await;
                    (|| -> Result<Option<usize>> {
                        if p.review.viewing().is_some() {
                            return Ok(None);
                        }
                        let before = p.head();
                        let mut repo = p.repo(&node.device)?;
                        let v = p.with_app(&node.device, |app| repo.save(app, p.binding().edit_message()))?;
                        if v.is_some() {
                            p.set_head(repo.head)?;
                            approve::sign_mine(&node, &p)?;
                        }
                        Ok(v.map(|_| p.photos_changed(before, p.head())))
                    })()
                };
                match r {
                    Ok(Some(photos)) => {
                        if let Some(h) = p.head() {
                            let _ = p.heads.send(h);
                        }
                        node.emit(json!({"event": "saved", "project": p.project.id.hex(), "head": p.head().map(|h| h.short()), "photos": photos, "now_ms": now_ms()}));
                        approve::notify(&node, &p);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        node.emit(json!({"event": "app", "project": p.project.id.hex(), "error": format!("{e:#}")}));
                        tokio::time::sleep(Duration::from_secs(4)).await;
                    }
                }
            }
        });
    }

    /// Start a project from what the app has now. `members`: account -> role and lanes.
    pub fn create_project(self: &Arc<Self>, name: &str, binding: Binding, members: BTreeMap<String, project::Member>) -> Result<Arc<Proj>> {
        self.create_project_mode(name, binding, members, None)
    }

    /// `create_project` with the project's mode: "live" (None) or "approve" (approve.rs).
    pub fn create_project_mode(self: &Arc<Self>, name: &str, binding: Binding, members: BTreeMap<String, project::Member>, mode: Option<&str>) -> Result<Arc<Proj>> {
        let tmp = self.device.home.join("projects").join(format!("new-{}", hex::encode(rand::random::<[u8; 4]>())));
        std::fs::create_dir_all(&tmp)?;
        let pid = {
            let store = RedbStore::open(tmp.join("kopano.redb"))?;
            let pr = Project::create_with_mode(&store, name, vec![self.device.cert.signer.clone()], &self.device.cert, &self.device.key, members, mode)?;
            pr.id
        };
        std::fs::write(tmp.join("project"), pid.hex())?;
        std::fs::write(tmp.join("binding.json"), serde_json::to_vec(&binding)?)?;
        std::fs::write(tmp.join("app.json"), b"{}")?;
        let dir = self.device.home.join("projects").join(&pid.hex()[..8]);
        std::fs::rename(&tmp, &dir)?;
        let p = Arc::new(Proj::open(&dir)?);
        if let Some(m) = self.media.get() {
            let _ = p.media.set(m.clone());
        }
        {
            let mut repo = p.repo(&self.device)?;
            p.with_app(&self.device, |app| repo.save(app, "Project started"))?;
            p.set_head(repo.head)?;
            approve::sign_mine(self, &p)?;
        }
        self.projects.write().unwrap().insert(pid, p.clone());
        self.watch_app(p.clone());
        Ok(p)
    }

    /// Join a project a teammate shared: fetch its signed genesis from them, check it, then watch
    /// them (the first push brings every version, and the app receives every item).
    pub async fn join(self: &Arc<Self>, pid: &str, from: &str, binding: Binding) -> Result<Arc<Proj>> {
        let id = Id::parse(pid)?;
        if let Ok(p) = self.project(pid) {
            return Ok(p);
        }
        let ep = self.endpoint.get().context("not started")?.clone();
        let eid = iroh::EndpointId::from_bytes(&hex::decode(from)?.try_into().map_err(|_| anyhow!("bad device id"))?)?;
        let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from(eid), ALPN)).await.map_err(|_| anyhow!("that teammate is not reachable"))??;
        let (mut s, mut r) = conn.open_bi().await?;
        write_frame(&mut s, &serde_json::to_vec(&self.device.cert)?).await?;
        write_frame(&mut s, pid.as_bytes()).await?;
        let first = read_frame(&mut r).await?;
        if first.starts_with(b"refused") {
            bail!("{}", String::from_utf8_lossy(&first));
        }
        write_frame(&mut s, &serde_json::to_vec(&json!({"op": "genesis"}))?).await?;
        let g = read_frame(&mut r).await?;
        conn.close(0u32.into(), b"joined");
        let dir = self.device.home.join("projects").join(&pid[..8]);
        std::fs::create_dir_all(&dir)?;
        {
            let store = RedbStore::open(dir.join("kopano.redb"))?;
            store.put_verified(&id, &g)?;
            store.flush()?;
            let pr = Project::load(&store, &id)?;
            let their: DeviceCert = serde_json::from_slice(&first)?;
            their.check(&pr.genesis.providers, now_ms())?;
            if their.body.device != from {
                bail!("the teammate's certificate is for another device");
            }
        }
        std::fs::write(dir.join("project"), pid)?;
        std::fs::write(dir.join("binding.json"), serde_json::to_vec(&binding)?)?;
        std::fs::write(dir.join("app.json"), b"{}")?;
        let p = Arc::new(Proj::open(&dir)?);
        if let Some(m) = self.media.get() {
            let _ = p.media.set(m.clone());
        }
        p.add_peer(from)?;
        self.projects.write().unwrap().insert(id, p.clone());
        self.follow(p.clone(), from.to_string());
        self.watch_app(p.clone());
        Ok(p)
    }

    /// Close the media store so what was just received is kept (E38).
    pub async fn close(&self) {
        if let Some(m) = self.media.get() {
            let _ = m.store.shutdown().await;
        }
    }

    pub fn add_teammate(self: &Arc<Self>, p: Arc<Proj>, device: &str) -> Result<()> {
        if hex::decode(device).map(|b| b.len()) != Ok(32) {
            bail!("a device id is 64 hex characters");
        }
        p.add_peer(device)?;
        self.follow(p, device.to_string());
        Ok(())
    }
}

/// E35: BBRv3 by default (34x faster at 1% loss, no cost on a clean link).
fn transport() -> Option<iroh::endpoint::QuicTransportConfig> {
    let cc: Arc<dyn iroh::endpoint::ControllerFactory + Send + Sync> = match std::env::var("KOPANO_CC").as_deref() {
        Ok("default") => return None,
        _ => Arc::new(noq_proto::congestion::Bbr3Config::default()),
    };
    Some(iroh::endpoint::QuicTransportConfig::builder().congestion_controller_factory(cc).build())
}

// ---- framing (same as the lab, incl. the slow-sender deadline from the security pass) ----------

async fn write_frame(s: &mut iroh::endpoint::SendStream, bytes: &[u8]) -> Result<()> {
    s.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    s.write_all(bytes).await?;
    Ok(())
}
async fn read_frame(r: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        bail!("frame of {n} bytes refused");
    }
    let deadline = Duration::from_secs(30 + (n / 16_384) as u64);
    let mut buf = vec![0u8; n];
    tokio::time::timeout(deadline, r.read_exact(&mut buf)).await.map_err(|_| anyhow!("frame of {n} bytes too slow"))??;
    Ok(buf)
}

// ---- server side: one handler for every project -------------------------------------------------

#[derive(Clone)]
struct Hub {
    node: Arc<Node>,
}
impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hub")
    }
}

/// Security pass (E26): strangers wait in a small room, oldest pushed out first; members share
/// MAX_MEMBERS slots; a certificate must arrive within HANDSHAKE_S.
const WAITING: usize = 16;
const HANDSHAKE_S: u64 = 5;
const MAX_MEMBERS: usize = 64;
static MEMBERS_IN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ROOM: std::sync::Mutex<std::collections::VecDeque<(u64, Connection)>> = std::sync::Mutex::new(std::collections::VecDeque::new());
static TICKETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn room_enter(conn: &Connection) -> u64 {
    let t = TICKETS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut room = ROOM.lock().unwrap_or_else(|e| e.into_inner());
    while room.len() >= WAITING {
        if let Some((_, old)) = room.pop_front() {
            old.close(2u32.into(), b"crowded");
        }
    }
    room.push_back((t, conn.clone()));
    t
}
fn room_leave(ticket: u64) {
    ROOM.lock().unwrap_or_else(|e| e.into_inner()).retain(|(t, _)| *t != ticket);
}

impl ProtocolHandler for Hub {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let ticket = room_enter(&conn);
        let mut member = false;
        if let Err(e) = self.serve(conn, ticket, &mut member).await {
            eprintln!("[kopanod] connection ended: {e:#}");
        }
        room_leave(ticket);
        if member {
            MEMBERS_IN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

impl Hub {
    async fn serve(&self, conn: Connection, ticket: u64, member: &mut bool) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let hello = async {
            let (send, mut recv) = conn.accept_bi().await?;
            let cert = read_frame(&mut recv).await?;
            let pid = read_frame(&mut recv).await?;
            anyhow::Ok((send, recv, cert, pid))
        };
        let (mut send, mut recv, cert, pid) = tokio::time::timeout(Duration::from_secs(HANDSHAKE_S), hello).await.map_err(|_| anyhow!("no certificate within {HANDSHAKE_S} s"))??;
        let their: DeviceCert = serde_json::from_slice(&cert)?;
        let p = match self.node.project(std::str::from_utf8(&pid)?).and_then(|p| p.check_peer(&their, &remote).map(|_| p)) {
            Ok(p) => p,
            Err(e) => {
                write_frame(&mut send, format!("refused: {e}").as_bytes()).await?;
                send.finish()?;
                let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
                bail!("refused {}: {e}", their.body.name);
            }
        };
        room_leave(ticket);
        if MEMBERS_IN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_MEMBERS {
            MEMBERS_IN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            conn.close(1u32.into(), b"busy");
            bail!("too many members connected");
        }
        *member = true;
        write_frame(&mut send, &serde_json::to_vec(&self.node.device.cert)?).await?;
        p.remember(&their);
        // a teammate who found us is one we watch back: one side adding the other is enough
        let dev = hex::encode(remote);
        if p.add_peer(&dev).unwrap_or(false) || !p.following.lock().unwrap().contains(&dev) {
            self.node.follow(p.clone(), dev.clone());
        }
        loop {
            let Ok(req) = read_frame(&mut recv).await else { break };
            let req: Value = serde_json::from_slice(&req)?;
            if let Some(boot) = req.get("boot").and_then(Value::as_u64) {
                // S8: its service restarted. Our own watch of it may hang on the dead connection
                // until QUIC gives up (45 s on this PC) or wait out a back-off: start it again
                // now. Only a restart changes `boot`, so two devices never ping-pong.
                if p.boots.lock().unwrap().insert(dev.clone(), boot).is_some_and(|old| old != boot) {
                    if let Some(k) = p.kicks.lock().unwrap().get(&dev) {
                        k.notify_waiters();
                    }
                }
            }
            match req.get("op").and_then(|o| o.as_str()) {
                Some("watch") if approve::on(&p) => {
                    let mut rx = p.heads.subscribe();
                    let mut pusher = approve::Pusher::new(&p, &req);
                    loop {
                        if let Some((note, z)) = pusher.next(&p)? {
                            write_frame(&mut send, &serde_json::to_vec(&note)?).await?;
                            write_frame(&mut send, &z).await?;
                        }
                        match rx.recv().await {
                            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                    break;
                }
                Some("watch") => {
                    let mut rx = p.heads.subscribe();
                    let mut known: Vec<Id> = req.get("have").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).filter(|h| p.store.has(h)).collect();
                    let mut next = p.head();
                    loop {
                        if let Some(h) = next {
                            if !known.contains(&h) {
                                let cached = p.pushed.lock().unwrap().get(&(known.clone(), h)).cloned();
                                let built = match cached {
                                    Some(b) => b,
                                    None => {
                                        let b = Arc::new(compact_payload(&p, &known)?);
                                        let mut c = p.pushed.lock().unwrap();
                                        if c.len() > 256 {
                                            c.clear();
                                        }
                                        c.insert((known.clone(), h), b.clone());
                                        b
                                    }
                                };
                                write_frame(&mut send, &serde_json::to_vec(&json!({"head": h.hex(), "ms": now_ms(), "records": built.0, "raw": built.1}))?).await?;
                                write_frame(&mut send, &built.2).await?;
                                known = vec![h];
                            }
                        }
                        match rx.recv().await {
                            Ok(h) => next = Some(h),
                            Err(broadcast::error::RecvError::Lagged(_)) => next = p.head(),
                            Err(_) => break,
                        }
                    }
                    break;
                }
                Some("genesis") => {
                    write_frame(&mut send, &p.store.get(&p.project.id)?).await?;
                }
                Some("media") => {
                    // a preview or an original of one of this project's photos (media.rs)
                    let reply = media::offer(&self.node, &p, &req).await;
                    write_frame(&mut send, &serde_json::to_vec(&reply)?).await?;
                }
                Some("tiles-packed") => {
                    // which packed form stands for which Photo tile here (photo_binding.rs)
                    let reply = photo_binding::packed_offer(&p, &req);
                    write_frame(&mut send, &serde_json::to_vec(&reply)?).await?;
                }
                Some("heads") => {
                    let heads: Vec<String> = p.head().into_iter().map(|h| h.hex()).collect();
                    write_frame(&mut send, &serde_json::to_vec(&json!({"heads": heads}))?).await?;
                }
                _ => bail!("unknown request"),
            }
        }
        Ok(())
    }
}

/// E19/E20 compact payload: versions the watcher lacks, oldest first, changed tree nodes as deltas
/// against the same place in the parent tree, all deflated. (records, raw bytes, compressed)
fn compact_payload(p: &Proj, have: &[Id]) -> Result<(u64, usize, Vec<u8>)> {
    compact_payload_of(p, have, &p.head().into_iter().collect::<Vec<_>>(), &[])
}

/// `compact_payload` for several heads, with `extra` objects (ref updates, members entries)
/// appended whole (approve.rs). Receivers store every record under its hash either way.
pub(crate) fn compact_payload_of(p: &Proj, have: &[Id], heads: &[Id], extra: &[Vec<u8>]) -> Result<(u64, usize, Vec<u8>)> {
    let known: BTreeSet<Id> = project::history(&p.store, have)?.into_iter().map(|(id, _)| id).collect();
    let mut versions: Vec<(Id, project::Version)> = project::history(&p.store, heads)?.into_iter().filter(|(id, _)| !known.contains(id)).collect();
    versions.reverse();
    let mut sent: HashSet<Id> = HashSet::new();
    let mut payload: Vec<u8> = Vec::new();
    let mut records = 0u64;
    let put = |kind: u8, data: &[u8], payload: &mut Vec<u8>| {
        payload.push(kind);
        payload.extend((data.len() as u32).to_le_bytes());
        payload.extend(data);
    };
    for (vid, v) in &versions {
        let parent_tree = match v.parents()?.first() {
            Some(pp) => project::load_version(&p.store, pp)?.tree()?,
            None => None,
        };
        let mut pairs = vec![];
        tree::delta_bases(&p.store, v.tree()?, parent_tree, &mut pairs)?;
        let mut nodes: HashSet<Id> = HashSet::new();
        for (nid, base) in &pairs {
            nodes.insert(*nid);
            if !sent.insert(*nid) {
                continue;
            }
            let nb = p.store.get(nid)?;
            match base.and_then(|b| p.store.get(&b).ok().and_then(|ob| tree::node_delta(&ob, &nb).map(|d| (b, d)))) {
                Some((b, d)) => {
                    let mut rec = b.0.to_vec();
                    rec.extend(d);
                    put(1, &rec, &mut payload);
                }
                None => put(0, &nb, &mut payload),
            }
            records += 1;
        }
        let mut miss = vec![];
        tree::missing(&p.store, v.tree()?, parent_tree, &mut miss)?;
        for id in miss {
            if nodes.contains(&id) || !sent.insert(id) {
                continue;
            }
            put(0, &p.store.get(&id)?, &mut payload);
            records += 1;
        }
        put(0, &p.store.get(vid)?, &mut payload);
        sent.insert(*vid);
        records += 1;
    }
    for b in extra {
        put(0, b, &mut payload);
        records += 1;
    }
    let z = miniz_oxide::deflate::compress_to_vec(&payload, 6);
    Ok((records, payload.len(), z))
}

// ---- local API for apps: one JSON request per line on 127.0.0.1, token required --------------

pub async fn serve_api(node: Arc<Node>, port: u16) -> Result<()> {
    serve_api_on(node, tokio::net::TcpListener::bind(("127.0.0.1", port)).await?).await
}

/// The API on a port already bound (kopanod binds it before going online, so a second kopanod
/// for this computer stops there instead of running the same identity twice).
pub async fn serve_api_on(node: Arc<Node>, l: tokio::net::TcpListener) -> Result<()> {
    let token = api_token(&node.device.home)?;
    loop {
        let (c, _) = l.accept().await?;
        let (node, token) = (node.clone(), token.clone());
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let (r, mut w) = c.into_split();
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if req.get("token").and_then(|t| t.as_str()) != Some(token.as_str()) {
                    let _ = w.write_all(b"{\"ok\":false,\"error\":\"bad token\"}\n").await;
                    break;
                }
                if req["op"] == "subscribe" {
                    // events until the app hangs up
                    let mut rx = node.events.subscribe();
                    let _ = w.write_all(b"{\"ok\":true,\"result\":\"subscribed\"}\n").await;
                    while let Ok(ev) = rx.recv().await {
                        if w.write_all(format!("{ev}\n").as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    return;
                }
                let out = match api(&node, &req).await {
                    Ok(v) => json!({"ok": true, "result": v}),
                    Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
                };
                if w.write_all(format!("{out}\n").as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
}

async fn api(node: &Arc<Node>, req: &Value) -> Result<Value> {
    let s = |k: &str| req.get(k).and_then(|v| v.as_str()).with_context(|| format!("{k} missing"));
    match s("op")? {
        "hello" => Ok(json!({"device": node.device.id(), "name": node.device.cert.body.name, "account": node.device.cert.body.account,
            "signed_in": node.device.cert.body.expires_ms > now_ms(), "signed_in_until_ms": node.device.cert.body.expires_ms,
            "projects": node.projects.read().unwrap().values().map(|p| json!({"id": p.project.id.hex(), "name": p.project.genesis.name, "app": p.binding()})).collect::<Vec<_>>()})),
        "status" => {
            let p = node.project(s("project")?)?;
            let (online, names) = (p.online(), p.names());
            let teammates: Vec<Value> = p.peers().into_iter().map(|d| {
                let n = names.get(&d).cloned().unwrap_or(Value::Null);
                json!({"device": d, "name": n.get("name"), "account": n.get("account"), "online": online.contains_key(&d)})
            }).collect();
            let members: Vec<Value> = p.project.genesis.members.iter().map(|(a, m)| json!({"account": a, "role": m.role, "owner": *a == p.project.genesis.owner})).collect();
            let mut out = json!({"head": p.head().map(|h| h.hex()), "name": p.project.genesis.name, "app": p.binding(), "teammates": teammates, "online": online, "members": members});
            if let (Some(o), Value::Object(extra)) = (out.as_object_mut(), approve::status_fields(node, &p)) {
                o.extend(extra);
            }
            if !approve::on(&p) {
                out["clashes"] = json!(approve::clashes(&p).unwrap_or_default());
            }
            Ok(out)
        }
        "get" => {
            let p = node.project(s("project")?)?;
            let app = p.app()?;
            let item = app.photos.get(s("key")?).context("no such item")?;
            Ok(item.state.pointer(&format!("/{}", s("field")?.replace('.', "/"))).cloned().unwrap_or(Value::Null))
        }
        "set" => {
            let p = node.project(s("project")?)?;
            let v = node.set(&p, s("key")?, s("field")?, req.get("value").cloned().unwrap_or(Value::Null)).await?;
            Ok(json!({"version": v.map(|v| v.short()), "ms": now_ms()}))
        }
        "create-project" => {
            let binding: Binding = serde_json::from_value(req.get("app").cloned().unwrap_or(json!({"kind": "toy"})))?;
            let mut members = BTreeMap::new();
            for m in req.get("members").and_then(|m| m.as_array()).into_iter().flatten() {
                let acct = m.get("account").and_then(|a| a.as_str()).context("member account")?;
                let role: project::Role = serde_json::from_value(m.get("role").cloned().unwrap_or(json!("editor")))?;
                let lanes: Vec<String> = serde_json::from_value(m.get("lanes").cloned().unwrap_or(json!([])))?;
                members.insert(acct.to_string(), project::Member { role, lanes });
            }
            // new projects default to "I approve changes"; "live" is today's shared line
            let mode = req.get("mode").and_then(|m| m.as_str()).unwrap_or("approve");
            let p = node.create_project_mode(s("name")?, binding, members, Some(mode))?;
            let snap = p.with_app(&node.device, |a| a.snapshot())?;
            let photos: BTreeSet<&str> = snap.keys().map(|k| k.rsplit_once('#').map(|(b, _)| b).unwrap_or(k)).collect();
            Ok(json!({"project": p.project.id.hex(), "head": p.head().map(|h| h.hex()), "items": snap.len(), "photos": photos.len(), "mode": if approve::on(&p) { "approve" } else { "live" }}))
        }
        "attach" => {
            let p = node.project(s("project")?)?;
            let binding: Binding = serde_json::from_value(req.get("app").cloned().context("app")?)?;
            p.rebind(binding)?;
            Ok(json!("attached"))
        }
        "join" => {
            let binding: Binding = serde_json::from_value(req.get("app").cloned().unwrap_or(json!({"kind": "toy"})))?;
            let p = node.join(s("project")?, s("from")?, binding).await?;
            Ok(json!({"project": p.project.id.hex()}))
        }
        "items" => {
            let p = node.project(s("project")?)?;
            let snap = p.with_app(&node.device, |a| a.snapshot())?;
            Ok(serde_json::to_value(snap.keys().collect::<Vec<_>>())?)
        }
        "media" => {
            let p = node.project(s("project")?)?;
            media::status(node, &p)
        }
        "fetch-original" => {
            let p = node.project(s("project")?)?;
            media::fetch_original(node, &p, s("key")?)
        }
        "pending" | "review" => {
            let p = node.project(s("project")?)?;
            if !approve::on(&p) {
                bail!("this project works live: there is nothing to approve");
            }
            approve::review(node, &p)
        }
        "approve" => {
            let p = node.project(s("project")?)?;
            if !approve::on(&p) {
                bail!("this project works live: there is nothing to approve");
            }
            let choices = req.get("choices").and_then(|c| c.as_object()).cloned().unwrap_or_default();
            approve::approve(node, &p, s("member")?, req.get("version").and_then(|v| v.as_str()), req.get("prefer").and_then(|v| v.as_str()), &choices).await
        }
        "decline" => {
            let p = node.project(s("project")?)?;
            if !approve::on(&p) {
                bail!("this project works live: there is nothing to decline");
            }
            approve::decline(node, &p, s("member")?, req.get("version").and_then(|v| v.as_str())).await
        }
        "view" => {
            let p = node.project(s("project")?)?;
            if !approve::on(&p) {
                bail!("this project works live: there is nothing to review");
            }
            let side = req.get("show").and_then(|v| v.as_str()).unwrap_or("member");
            if !["member", "main", "off"].contains(&side) {
                bail!("show is member, main or off");
            }
            let who = if side == "off" { "" } else { s("member")? };
            approve::view(node, &p, who, req.get("version").and_then(|v| v.as_str()), side).await
        }
        "set-approver" => {
            let p = node.project(s("project")?)?;
            if !approve::on(&p) {
                bail!("this project works live: everyone's edits go to everyone");
            }
            approve::set_approver(node, &p, s("account")?, req.get("approver").and_then(|v| v.as_bool()).unwrap_or(true)).await
        }
        "clashes" => {
            let p = node.project(s("project")?)?;
            Ok(json!(approve::clashes(&p)?))
        }
        "resolve" => {
            let p = node.project(s("project")?)?;
            let pick = s("pick")?;
            if pick != "a" && pick != "b" {
                bail!("pick is a or b");
            }
            approve::resolve(node, &p, s("clash")?, pick).await
        }
        "add-teammate" => {
            let p = node.project(s("project")?)?;
            node.add_teammate(p, s("device")?)?;
            Ok(json!("following"))
        }
        o => bail!("unknown op {o}"),
    }
}

/// Every state (item id) the project held for each of `keys`, in any version under `heads`: what
/// a window that is linked again may hold without it being new work (`Proj::reconcile`).
pub(crate) fn seen_states<'a>(store: &dyn ObjectStore, heads: &[Id], keys: impl Iterator<Item = &'a String>) -> Result<HashMap<String, HashSet<Id>>> {
    let keys: Vec<&String> = keys.collect();
    let mut out: HashMap<String, HashSet<Id>> = HashMap::new();
    let mut done: HashSet<Id> = HashSet::new();
    for (i, h) in heads.iter().enumerate() {
        // the head's history must be whole; another ref's may still be on its way: skip it
        let hist = match project::history(store, &[*h]) {
            Ok(x) => x,
            Err(e) if i == 0 => return Err(e),
            Err(_) => continue,
        };
        for (vid, v) in hist {
            if !done.insert(vid) {
                continue;
            }
            let root = v.tree()?;
            for k in &keys {
                if let Some(id) = tree::get(store, root, k)? {
                    out.entry((*k).clone()).or_default().insert(id);
                }
            }
        }
    }
    Ok(out)
}

/// The token apps must show. Created once, readable by this user only (a file next to the key).
fn unreachable_toy() -> Result<LightConnector> {
    bail!("this project is not bound to Light")
}

pub fn api_token(home: &Path) -> Result<String> {
    let f = home.join("api.token");
    if let Ok(t) = std::fs::read_to_string(&f) {
        return Ok(t.trim().to_string());
    }
    let t = hex::encode(rand::random::<[u8; 24]>());
    std::fs::write(&f, &t)?;
    Ok(t)
}

/// Move one lab peer folder in: the first one also gives the home its device key and certificate.
pub fn import_peer_dir(home: &Path, from: &Path) -> Result<String> {
    std::fs::create_dir_all(home.join("projects"))?;
    for f in ["device.key", "cert.json"] {
        let (src, dst) = (from.join(f), home.join(f));
        if dst.exists() {
            if std::fs::read(&src)? != std::fs::read(&dst)? {
                bail!("{f} differs: that folder belongs to another device");
            }
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    let pid = std::fs::read_to_string(from.join("project"))?.trim().to_string();
    let to = home.join("projects").join(&pid[..8]);
    std::fs::create_dir_all(&to)?;
    for f in ["kopano.redb", "project", "head", "app.json"] {
        if from.join(f).exists() {
            std::fs::copy(from.join(f), to.join(f))?;
        }
    }
    Ok(pid)
}
