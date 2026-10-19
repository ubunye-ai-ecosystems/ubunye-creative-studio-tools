//! kopanod engine: one device identity, many projects, live sync both ways (09, slice S1).
//!
//! Why one service per computer: one device key must never run in two programs (E37c: two helpers
//! with one id answered each other's requests; E39: the relay refused a second endpoint with the
//! same key). Every app (Light, Photo, Film) talks to this service instead of opening its own.
//!
//! Home folder:
//!   device.key, cert.json          this device (key in the OS key store when protected)
//!   api.token                      apps must show it on the local API
//!   projects/<id8>/                kopano.redb, project, head, app.json, peers.json
//!
//! Sync protocol `kopano/sync/1` = the lab's `kopano/sync/0` plus one frame after the certificate
//! naming the project, so one endpoint serves every project. Live edits travel as compact pushes
//! (E20/E24): each side WATCHES each teammate device it knows and is watched by them, so an edit
//! reaches the other side in about half a round trip with no pull command.

#[path = "../../kopano-lab/src/keystore.rs"]
pub mod keystore;

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
    online: std::sync::Mutex<BTreeMap<String, String>>,
    pub binding: Binding,
    light: std::sync::Mutex<Option<LightConnector>>,
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
            online: Default::default(),
            binding: std::fs::read(dir.join("binding.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Binding::Toy),
            light: Default::default(),
        })
    }
    /// Run `f` against this project's app: the toy editor file, or Light (connected on first use
    /// and again after a dropped connection).
    fn with_app<R>(&self, f: impl FnOnce(&mut dyn Connector) -> Result<R>) -> Result<R> {
        tokio::task::block_in_place(|| match &self.binding {
            Binding::Toy => {
                let mut app = self.app()?;
                let r = f(&mut app)?;
                self.save_app(&app)?;
                Ok(r)
            }
            b => {
                let mut g = self.light.lock().unwrap_or_else(|e| e.into_inner());
                if g.is_none() {
                    *g = Some(match b {
                        Binding::LightControl { port } => LightConnector::live(*port).context("Light is not running with its control port")?,
                        Binding::LightHeadless { cli } => LightConnector::headless(cli)?,
                        Binding::Toy => unreachable_toy()?,
                    });
                }
                let r = f(g.as_mut().context("no app")?);
                if r.is_err() {
                    *g = None; // reconnect next time
                }
                r
            }
        })
    }
    pub fn head(&self) -> Option<Id> {
        std::fs::read_to_string(self.dir.join("head")).ok().and_then(|s| Id::parse(s.trim()).ok())
    }
    fn set_head(&self, id: Option<Id>) -> Result<()> {
        match id {
            Some(h) => kopano_core::store::write_atomic(&self.dir.join("head"), h.hex().as_bytes()),
            None => Ok(()),
        }
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
        if !self.project.genesis.members.contains_key(&cert.body.account) {
            bail!("{} is not a member of this project", cert.body.name);
        }
        Ok(())
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
}

impl Node {
    pub fn open(home: &Path) -> Result<Arc<Self>> {
        let device = Device::open(home)?;
        let mut projects = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(home.join("projects")) {
            for e in rd.flatten() {
                match Proj::open(&e.path()) {
                    Ok(p) => {
                        projects.insert(p.project.id, Arc::new(p));
                    }
                    Err(err) => eprintln!("[kopanod] skipped {}: {err:#}", e.path().display()),
                }
            }
        }
        Ok(Arc::new(Self { device, projects: std::sync::RwLock::new(projects), events: broadcast::channel(1024).0, endpoint: Default::default() }))
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
        let mut b = Endpoint::builder(presets::N0).secret_key(sk).alpns(vec![ALPN.to_vec()]);
        if let Some(t) = transport() {
            b = b.transport_config(t);
        }
        if let Ok(u) = std::env::var("KOPANO_RELAY_URL") {
            let url: iroh::RelayUrl = u.parse().context("KOPANO_RELAY_URL")?;
            b = b.relay_mode(iroh::endpoint::RelayMode::Custom(iroh::RelayMap::from(url)));
        }
        let ep = b.bind().await?;
        let router = Router::builder(ep.clone()).accept(ALPN, Hub { node: self.clone() }).spawn();
        let _ = tokio::time::timeout(Duration::from_secs(10), ep.online()).await;
        let _ = self.endpoint.set(ep);
        let ps: Vec<Arc<Proj>> = self.projects.read().unwrap().values().cloned().collect();
        for p in ps {
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
        tokio::spawn(async move {
            let mut wait = 1u64;
            loop {
                let started = Instant::now();
                if let Err(e) = node.watch(&p, &device).await {
                    if p.online.lock().unwrap().remove(&device).is_some() {
                        node.emit(json!({"event": "teammate", "project": p.project.id.hex(), "device": device, "online": false}));
                    }
                    eprintln!("[kopanod] {} on {}: {e:#}; retrying in {wait} s", &device[..8], p.project.id.short());
                }
                if started.elapsed().as_secs() > 60 {
                    wait = 1;
                }
                tokio::time::sleep(Duration::from_secs(wait)).await;
                wait = (wait * 2).min(30);
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
        self.emit(json!({"event": "teammate", "project": p.project.id.hex(), "device": device, "name": their.body.name, "online": true}));
        let have: Vec<String> = p.head().into_iter().map(|h| h.hex()).collect();
        write_frame(&mut s, &serde_json::to_vec(&json!({"op": "watch", "have": have}))?).await?;
        loop {
            let note: Value = serde_json::from_slice(&read_frame(&mut r).await?)?;
            let z = read_frame(&mut r).await?;
            let Some(h) = note.get("head").and_then(|h| h.as_str()).and_then(|h| Id::parse(h).ok()) else { continue };
            let t = Instant::now();
            let raw = note.get("raw").and_then(|n| n.as_u64()).unwrap_or(0);
            let outcome = {
                let _w = p.write.lock().await;
                let versions = kopano_core::wire::decode_compact(&p.store, &z, raw)?.versions;
                for v in &versions {
                    project::verify_version(&p.store, &p.project, v)?;
                }
                self.integrate(p, h)?
            };
            if let Some((what, conflicts)) = outcome {
                if let Some(nh) = p.head() {
                    let _ = p.heads.send(nh);
                }
                self.emit(json!({"event": "synced", "project": p.project.id.hex(), "from": their.body.name, "head": p.head().map(|h| h.short()),
                    "how": what, "clashes": conflicts, "bytes": z.len(), "apply_ms": t.elapsed().as_millis() as u64,
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
                    if theirs < m {
                        p.set_head(Some(theirs))?;
                        return Ok(Some(("same content, settled", 0)));
                    }
                    return Ok(None);
                }
            }
        }
        let mut repo = p.repo(&self.device)?;
        let r = p.with_app(|app| repo.integrate(app, &theirs))?;
        p.set_head(repo.head)?;
        p.store.flush()?;
        Ok(match r {
            Integrated::AlreadyHad => None,
            Integrated::FastForward { .. } => Some(("fast-forward", 0)),
            Integrated::Merged { conflicts, .. } => Some(("merged", conflicts.len())),
        })
    }

    /// A local edit from an app: save a version and push it to every watcher.
    pub async fn set(&self, p: &Proj, key: &str, field: &str, value: Value) -> Result<Option<Id>> {
        let v = {
            let _w = p.write.lock().await;
            if p.binding != Binding::Toy {
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
            v
        };
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
        if p.binding == Binding::Toy {
            return;
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
                let r = {
                    let _w = p.write.lock().await;
                    (|| -> Result<Option<Id>> {
                        let mut repo = p.repo(&node.device)?;
                        let v = p.with_app(|app| repo.save(app, "Edit in Light"))?;
                        if v.is_some() {
                            p.set_head(repo.head)?;
                        }
                        Ok(v)
                    })()
                };
                match r {
                    Ok(Some(_)) => {
                        if let Some(h) = p.head() {
                            let _ = p.heads.send(h);
                        }
                        node.emit(json!({"event": "saved", "project": p.project.id.hex(), "head": p.head().map(|h| h.short()), "now_ms": now_ms()}));
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
        let tmp = self.device.home.join("projects").join(format!("new-{}", hex::encode(rand::random::<[u8; 4]>())));
        std::fs::create_dir_all(&tmp)?;
        let pid = {
            let store = RedbStore::open(tmp.join("kopano.redb"))?;
            let pr = Project::create(&store, name, vec![self.device.cert.signer.clone()], &self.device.cert, &self.device.key, members)?;
            pr.id
        };
        std::fs::write(tmp.join("project"), pid.hex())?;
        std::fs::write(tmp.join("binding.json"), serde_json::to_vec(&binding)?)?;
        std::fs::write(tmp.join("app.json"), b"{}")?;
        let dir = self.device.home.join("projects").join(&pid.hex()[..8]);
        std::fs::rename(&tmp, &dir)?;
        let p = Arc::new(Proj::open(&dir)?);
        {
            let mut repo = p.repo(&self.device)?;
            p.with_app(|app| repo.save(app, "Project started"))?;
            p.set_head(repo.head)?;
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
        p.add_peer(from)?;
        self.projects.write().unwrap().insert(id, p.clone());
        self.follow(p.clone(), from.to_string());
        self.watch_app(p.clone());
        Ok(p)
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
        // a teammate who found us is one we watch back: one side adding the other is enough
        let dev = hex::encode(remote);
        if p.add_peer(&dev).unwrap_or(false) || !p.following.lock().unwrap().contains(&dev) {
            self.node.follow(p.clone(), dev.clone());
        }
        loop {
            let Ok(req) = read_frame(&mut recv).await else { break };
            let req: Value = serde_json::from_slice(&req)?;
            match req.get("op").and_then(|o| o.as_str()) {
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
    let known: BTreeSet<Id> = project::history(&p.store, have)?.into_iter().map(|(id, _)| id).collect();
    let mut versions: Vec<(Id, project::Version)> = project::history(&p.store, &p.head().into_iter().collect::<Vec<_>>())?.into_iter().filter(|(id, _)| !known.contains(id)).collect();
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
    let z = miniz_oxide::deflate::compress_to_vec(&payload, 6);
    Ok((records, payload.len(), z))
}

// ---- local API for apps: one JSON request per line on 127.0.0.1, token required --------------

pub async fn serve_api(node: Arc<Node>, port: u16) -> Result<()> {
    let token = api_token(&node.device.home)?;
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
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
            "projects": node.projects.read().unwrap().values().map(|p| json!({"id": p.project.id.hex(), "name": p.project.genesis.name})).collect::<Vec<_>>()})),
        "status" => {
            let p = node.project(s("project")?)?;
            Ok(json!({"head": p.head().map(|h| h.hex()), "items": p.app()?.photos.len(), "teammates": p.peers(), "online": p.online()}))
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
            let p = node.create_project(s("name")?, binding, members)?;
            let n = p.with_app(|a| Ok(a.snapshot()?.len()))?;
            Ok(json!({"project": p.project.id.hex(), "head": p.head().map(|h| h.hex()), "items": n}))
        }
        "join" => {
            let binding: Binding = serde_json::from_value(req.get("app").cloned().unwrap_or(json!({"kind": "toy"})))?;
            let p = node.join(s("project")?, s("from")?, binding).await?;
            Ok(json!({"project": p.project.id.hex()}))
        }
        "items" => {
            let p = node.project(s("project")?)?;
            let snap = p.with_app(|a| a.snapshot())?;
            Ok(serde_json::to_value(snap.keys().collect::<Vec<_>>())?)
        }
        "add-teammate" => {
            let p = node.project(s("project")?)?;
            node.add_teammate(p, s("device")?)?;
            Ok(json!("following"))
        }
        o => bail!("unknown op {o}"),
    }
}

/// The token apps must show. Created once, readable by this user only (a file next to the key).
fn unreachable_toy() -> Result<LightConnector> {
    bail!("toy binding has no Light connection")
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
