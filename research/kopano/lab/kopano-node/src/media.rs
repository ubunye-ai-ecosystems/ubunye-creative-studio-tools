//! kopanod media: photos travel to a teammate who does not have them (slice "media").
//!
//! Edits move as small items; the files they are about move separately, only when needed, as
//! BLAKE3-addressed blobs over the same endpoint (iroh-blobs ALPN), and only to the devices of
//! the project's members:
//!
//! 1. Preview first. A photo in the project that this device's app does not show yet gets a
//!    preview from whichever online teammate has one (the device with the original makes it
//!    once, in its own app's renderer, and keeps it to hand out again). The preview is imported
//!    into the app and stands in for the original under the ORIGINAL's photo key, so every edit
//!    made on it lands on the same photo everywhere.
//! 2. Original on request (`fetch-original`): the full file, with progress, resume after a cut
//!    and a disk-space check first; then it is imported beside the preview, given the project's
//!    settings, and the preview is taken out.
//!
//! Home folder: `media/store` (blobs), `media/previews.json` (previews this device made, by
//! original), `media/previews/<project>/` and `media/originals/<project>/` (files the app
//! shows). Per project: `media.json` (photo key -> the file standing in for it here).
//!
//! The keys (why they match on both sides): Light keys a photo by its own content hash of the
//! file it imported. The preview is another file, so Light would key it differently. kopanod
//! therefore tells the Light connector which preview file stands for which project key
//! (`PreviewAlias`, by file path, which survives Light restarts and asynchronous imports), and
//! the connector reports the photo under that key with the original's asset reference. A
//! preview file kopanod did not map is never published as a photo, and once the original is in
//! Light beside its preview the original wins.

use crate::{read_frame, write_frame, Binding, Node, Proj, ALPN};
use anyhow::{anyhow, bail, Context, Result};
use iroh::EndpointAddr;
use iroh_blobs::store::fs::FsStore;
use kopano_core::connector::{Connector, ConnectorInfo};
use kopano_core::identity::{now_ms, DeviceCert};
use kopano_core::project::{self, AssetRef, Item};
use kopano_core::store::{Id, RedbStore};
use kopano_core::tree;
use kopano_lab::light::{norm_path, LightConnector, PreviewAlias};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A file a device can hand out: what an asker needs to fetch and name it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Offer {
    pub hash: String,
    pub size: u64,
    /// the original's file name (a preview is named after it)
    pub name: String,
}

/// The file standing in for one photo on this device.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Local {
    /// "preview" or "original"
    pub kind: String,
    pub path: String,
    pub name: String,
    pub original: AssetRef,
    /// BLAKE3 of the preview (also handed on to teammates who ask)
    #[serde(default)]
    pub preview: Option<String>,
}

pub struct Media {
    pub store: FsStore,
    dir: PathBuf,
    /// previews this device made from its own originals: original BLAKE3 -> offer
    made: std::sync::Mutex<BTreeMap<String, Offer>>,
    /// one render at a time: a 24 MP raw takes ~3 s and a few hundred MB
    render: tokio::sync::Mutex<()>,
    /// blobs connection id -> device, for the per-request check
    conns: std::sync::Mutex<HashMap<u64, [u8; 32]>>,
    /// per project: (head, every media hash the project lets members fetch)
    refs: std::sync::Mutex<HashMap<Id, (Option<Id>, HashSet<String>)>>,
    /// originals on their way, for the `media` op: key -> progress
    pub jobs: std::sync::Mutex<BTreeMap<String, Value>>,
    /// making previews ahead failed once (e.g. no Light engine here): stop trying in the
    /// background; a teammate's request still tries and gets the reason
    ahead_off: std::sync::atomic::AtomicBool,
}

impl Media {
    pub async fn open(home: &Path) -> Result<Arc<Self>> {
        let dir = home.join("media");
        std::fs::create_dir_all(dir.join("previews"))?;
        let store = FsStore::load(dir.join("store")).await?;
        let made = std::fs::read(dir.join("previews.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Ok(Arc::new(Self { store, dir, made: std::sync::Mutex::new(made), render: Default::default(), conns: Default::default(), refs: Default::default(), jobs: Default::default(), ahead_off: Default::default() }))
    }
    pub fn preview_dir(&self) -> PathBuf {
        self.dir.join("previews")
    }
    async fn complete(&self, hash: &str) -> bool {
        match hash.parse::<iroh_blobs::Hash>() {
            Ok(h) => self.store.remote().local(h).await.map(|l| l.is_complete()).unwrap_or(false),
            Err(_) => false,
        }
    }
    fn remember_made(&self, original: &str, o: Offer) -> Result<()> {
        let mut m = self.made.lock().unwrap_or_else(|e| e.into_inner());
        m.insert(original.into(), o);
        kopano_core::store::write_atomic(&self.dir.join("previews.json"), &serde_json::to_vec(&*m)?)
    }
}

// ---- per project: which file stands in for which photo here ------------------------------------

pub fn local(p: &Proj) -> BTreeMap<String, Local> {
    std::fs::read(p.dir.join("media.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn save_local(p: &Proj, m: &BTreeMap<String, Local>) -> Result<()> {
    kopano_core::store::write_atomic(&p.dir.join("media.json"), &serde_json::to_vec_pretty(m)?)
}
/// What the Light connector needs to show previews under the originals' keys.
pub fn aliases(m: &BTreeMap<String, Local>) -> BTreeMap<String, PreviewAlias> {
    m.iter().filter(|(_, l)| l.kind == "preview").map(|(k, l)| (norm_path(&l.path), PreviewAlias { key: k.clone(), assets: vec![l.original.clone()] })).collect()
}
/// Give a freshly connected Light window this project's previews.
pub fn prepare(node_media: Option<&Arc<Media>>, p: &Proj, c: &mut LightConnector) {
    c.previews = aliases(&local(p));
    c.preview_dir = node_media.map(|m| m.preview_dir().to_string_lossy().to_string());
}

fn base(k: &str) -> &str {
    k.rsplit_once('#').map(|(b, _)| b).unwrap_or(k)
}

/// The photos of the project's head (base key -> its original), from the develop items.
fn originals_in(store: &RedbStore, head: Option<Id>) -> Result<BTreeMap<String, AssetRef>> {
    let mut out = BTreeMap::new();
    let Some(h) = head else { return Ok(out) };
    for (k, id) in tree::entries(store, project::load_version(store, &h)?.tree()?)? {
        if !k.ends_with("#develop") {
            continue;
        }
        if let Some(a) = Item::get(store, &id)?.assets.into_iter().find(|a| a.role == "original") {
            out.insert(base(&k).to_string(), a);
        }
    }
    Ok(out)
}

// ---- the app as kopanod sees it: photos not here yet are not "removed" ------------------------

/// An app that can say whether it shows a photo (its key before the `#lane`).
pub trait Shows: Connector {
    fn shows(&self, key: &str) -> bool;
}
impl Shows for LightConnector {
    fn shows(&self, key: &str) -> bool {
        self.has_photo(key)
    }
}

/// Light as the project sees it. A photo the window does not show yet (its preview is on the
/// way) still counts as the project's state, not as deleted: without this a teammate whose
/// Light starts empty would save "everything removed" and send that to the team. Changes for
/// such a photo wait in the project; they are applied when its preview arrives.
pub struct View<'a, A: Shows + ?Sized> {
    store: &'a RedbStore,
    head: Option<Id>,
    pub app: &'a mut A,
}

impl<'a, A: Shows + ?Sized> View<'a, A> {
    pub fn new(store: &'a RedbStore, head: Option<Id>, app: &'a mut A) -> Self {
        Self { store, head, app }
    }
}

impl<A: Shows + ?Sized> Connector for View<'_, A> {
    fn info(&self) -> ConnectorInfo {
        self.app.info()
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        let mut s = self.app.snapshot()?;
        let Some(h) = self.head else { return Ok(s) };
        let info = self.app.info();
        for (k, id) in tree::entries(self.store, project::load_version(self.store, &h)?.tree()?)? {
            if s.contains_key(&k) || self.app.shows(&k) {
                continue;
            }
            let item = Item::get(self.store, &id)?;
            if info.owns(&item.kind) {
                s.insert(k, item);
            }
        }
        Ok(s)
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        if !self.app.shows(key) {
            return Ok(()); // waits in the project for its preview
        }
        self.app.apply(key, item)
    }
}

/// Photos of the head that the window does not show (base key -> original).
fn absent(p: &Proj, c: &mut LightConnector) -> Result<BTreeMap<String, AssetRef>> {
    c.snapshot()?;
    Ok(originals_in(&p.store, p.head())?.into_iter().filter(|(k, _)| !c.has_photo(k)).collect())
}

/// Give the window the project's state for these photos (they just arrived in it).
fn apply_head(p: &Proj, c: &mut LightConnector, keys: &BTreeSet<String>) -> Result<usize> {
    let Some(h) = p.head() else { return Ok(0) };
    let mut n = 0;
    for (k, id) in tree::entries(&p.store, project::load_version(&p.store, &h)?.tree()?)? {
        if keys.contains(base(&k)) && c.info().owns(&Item::get(&p.store, &id)?.kind) {
            c.apply(&k, Some(&Item::get(&p.store, &id)?))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Wait (in the blocking section) until Light shows these photos: imports run on a worker thread.
fn wait_shown(c: &mut LightConnector, keys: &BTreeSet<String>, original: bool, secs: u64) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        c.snapshot()?;
        let missing: Vec<&String> = keys.iter().filter(|k| if original { !c.has_original(k) } else { !c.has_photo(k) }).collect();
        if missing.is_empty() {
            return Ok(());
        }
        if Instant::now() > end {
            bail!("Light did not show {} photo(s) within {secs} s of importing them", missing.len());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ---- who may fetch what ------------------------------------------------------------------------

/// A device that proved a member's certificate on this project and whose certificate has not
/// run out (kopanod saw it in the sync handshake; names.json keeps it).
pub fn member_device(p: &Proj, device: &str) -> bool {
    let names = p.names();
    let Some(n) = names.get(device) else { return false };
    let acct = n.get("account").and_then(Value::as_str).unwrap_or("");
    let fresh = n.get("expires_ms").and_then(Value::as_u64).map_or(true, |e| e > now_ms());
    p.peers().contains(device) && fresh && p.project.genesis.members.contains_key(acct)
}

/// Every media hash this project lets its members fetch: the originals its items reference and
/// the previews of those originals (made here, or received here and handed on).
fn refs(m: &Media, p: &Proj) -> HashSet<String> {
    let head = p.head();
    if let Some((h, set)) = m.refs.lock().unwrap_or_else(|e| e.into_inner()).get(&p.project.id) {
        if *h == head {
            return set.clone();
        }
    }
    let originals: HashSet<String> = originals_in(&p.store, head).map(|o| o.into_values().map(|a| a.hash).collect()).unwrap_or_default();
    let mut set = originals.clone();
    for (orig, o) in m.made.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if originals.contains(orig) {
            set.insert(o.hash.clone());
        }
    }
    set.extend(local(p).into_values().filter_map(|l| l.preview));
    m.refs.lock().unwrap_or_else(|e| e.into_inner()).insert(p.project.id, (head, set.clone()));
    set
}

/// May `device` fetch `hash`? Only a current member's device, and only media of a project it
/// is a member of: knowing a hash is not enough.
pub fn may_fetch(node: &Node, device: &[u8; 32], hash: &str) -> bool {
    let (dev, Some(m)) = (hex::encode(device), node.media.get()) else { return false };
    let ps: Vec<Arc<Proj>> = node.projects.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
    ps.iter().any(|p| member_device(p, &dev) && refs(m, p).contains(hash))
}

/// iroh-blobs hook: refuse strangers at connect, and check every request against the projects.
pub fn gate(node: Arc<Node>) -> iroh_blobs::provider::events::EventSender {
    use iroh_blobs::provider::events::{AbortReason, ConnectMode, EventMask, EventSender, ProviderMessage, RequestMode};
    let mask = EventMask { connected: ConnectMode::Intercept, get: RequestMode::Intercept, get_many: RequestMode::Disabled, ..EventMask::DEFAULT };
    let (tx, mut rx) = EventSender::channel(64, mask);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                ProviderMessage::ClientConnected(msg) => {
                    let dev = msg.endpoint_id.map(|i| *i.as_bytes());
                    let ok = dev.is_some_and(|d| {
                        let h = hex::encode(d);
                        node.projects.read().unwrap_or_else(|e| e.into_inner()).values().any(|p| member_device(p, &h))
                    });
                    if let (true, Some(d), Some(m)) = (ok, dev, node.media.get()) {
                        m.conns.lock().unwrap_or_else(|e| e.into_inner()).insert(msg.connection_id, d);
                    }
                    if !ok {
                        eprintln!("[kopanod] media: refused {}", dev.map(|d| hex::encode(&d[..4])).unwrap_or_default());
                    }
                    msg.tx.send(if ok { Ok(()) } else { Err(AbortReason::Permission) }).await.ok();
                }
                ProviderMessage::GetRequestReceived(msg) => {
                    let dev = node.media.get().and_then(|m| m.conns.lock().unwrap_or_else(|e| e.into_inner()).get(&msg.connection_id).copied());
                    let hash = msg.request.hash.to_hex().to_string();
                    let ok = dev.is_some_and(|d| may_fetch(&node, &d, &hash));
                    if !ok {
                        eprintln!("[kopanod] media: refused a request for {}", &hash[..8]);
                    }
                    msg.tx.send(if ok { Ok(()) } else { Err(AbortReason::Permission) }).await.ok();
                }
                ProviderMessage::ConnectionClosed(msg) => {
                    if let Some(m) = node.media.get() {
                        m.conns.lock().unwrap_or_else(|e| e.into_inner()).remove(&msg.connection_id);
                    }
                }
                _ => {}
            }
        }
    });
    tx
}

// ---- serving: hand out a preview or an original -------------------------------------------------

/// The Light command-line engine that renders previews: KOPANO_LIGHT_CLI, else next to
/// kopanod, else on PATH.
pub fn light_cli() -> String {
    if let Ok(c) = std::env::var("KOPANO_LIGHT_CLI") {
        if !c.is_empty() {
            return c;
        }
    }
    let exe = if cfg!(windows) { "lightcraft-cli.exe" } else { "lightcraft-cli" };
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(exe))).filter(|p| p.is_file()).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| exe.into())
}

/// Answer a member's `{"op":"media", "hash", "role": "preview"|"original"}`.
pub async fn offer(node: &Arc<Node>, p: &Arc<Proj>, req: &Value) -> Value {
    match offer_inner(node, p, req).await {
        Ok(o) => serde_json::to_value(o).unwrap_or(Value::Null),
        Err(e) => json!({"error": format!("{e:#}")}),
    }
}

async fn offer_inner(node: &Arc<Node>, p: &Arc<Proj>, req: &Value) -> Result<Offer> {
    let m = node.media.get().context("media not started")?;
    let hash = req.get("hash").and_then(Value::as_str).context("hash")?.to_string();
    let originals = originals_in(&p.store, p.head())?;
    let Some(asset) = originals.values().find(|a| a.hash == hash).cloned() else { bail!("that is not a photo of this project") };
    let light = p.binding() != Binding::Toy;
    let path = if light { p.with_light(&node.device, |c| Ok(c.original_path(&hash))).ok().flatten() } else { None };
    let name_of = |path: &str| Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let here = local(p).into_values().find(|l| l.original.hash == hash);
    match req.get("role").and_then(Value::as_str) {
        Some("original") => {
            let name = path.as_deref().map(name_of).or(here.as_ref().map(|l| l.name.clone())).unwrap_or_default();
            if m.complete(&hash).await {
                return Ok(Offer { hash, size: asset.size, name });
            }
            let Some(path) = path else { bail!("the original is not on this computer") };
            // in place: a 49 MB raw is not copied into the store, only indexed
            let opts = iroh_blobs::api::blobs::AddPathOptions { path: PathBuf::from(&path), format: iroh_blobs::BlobFormat::Raw, mode: iroh_blobs::api::blobs::ImportMode::TryReference };
            let tag = m.store.blobs().add_path_with_opts(opts).await?;
            if tag.hash.to_hex().to_string() != hash {
                bail!("{path} changed since it was added to the project");
            }
            Ok(Offer { hash, size: asset.size, name })
        }
        Some("preview") => {
            let made = m.made.lock().unwrap_or_else(|e| e.into_inner()).get(&hash).cloned();
            if let Some(o) = made {
                if m.complete(&o.hash).await {
                    return Ok(o);
                }
            }
            if let Some(l) = &here {
                if let Some(ph) = &l.preview {
                    if m.complete(ph).await {
                        let size = m.store.remote().local(ph.parse::<iroh_blobs::Hash>()?).await?.local_bytes();
                        return Ok(Offer { hash: ph.clone(), size, name: l.name.clone() });
                    }
                }
            }
            let Some(path) = path else { bail!("no preview of this photo on this computer") };
            let _one = m.render.lock().await;
            // made meanwhile (ahead, or for another teammate) while this one waited its turn
            let made = m.made.lock().unwrap_or_else(|e| e.into_inner()).get(&hash).cloned();
            if let Some(o) = made {
                if m.complete(&o.hash).await {
                    return Ok(o);
                }
            }
            let out = m.dir.join(format!("made-{}.jpg", &hash[..16]));
            let (cli, src, dst) = (light_cli(), PathBuf::from(&path), out.clone());
            let t = Instant::now();
            tokio::task::spawn_blocking(move || kopano_lab::light::make_preview(&cli, &src, &dst)).await??;
            let render_ms = t.elapsed().as_millis() as u64;
            let tag = m.store.blobs().add_path(&out).await?;
            let size = std::fs::metadata(&out)?.len();
            let _ = std::fs::remove_file(&out);
            let o = Offer { hash: tag.hash.to_hex().to_string(), size, name: name_of(&path) };
            m.remember_made(&hash, o.clone())?;
            m.refs.lock().unwrap_or_else(|e| e.into_inner()).remove(&p.project.id);
            node.emit(json!({"event": "preview-made", "project": p.project.id.hex(), "photo": o.name, "kb": size / 1000, "ms": render_ms}));
            Ok(o)
        }
        _ => bail!("role must be preview or original"),
    }
}

// ---- asking: one request to a teammate, then the blob ------------------------------------------

/// One request on the sync protocol (same handshake as watch and join).
async fn ask(node: &Node, p: &Proj, device: &str, req: &Value) -> Result<Value> {
    let ep = node.endpoint.get().context("not started")?.clone();
    let id = iroh::EndpointId::from_bytes(&hex::decode(device)?.try_into().map_err(|_| anyhow!("bad device id"))?)?;
    let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from(id), ALPN)).await.map_err(|_| anyhow!("not reachable"))??;
    let (mut s, mut r) = conn.open_bi().await?;
    write_frame(&mut s, &serde_json::to_vec(&node.device.cert)?).await?;
    write_frame(&mut s, p.project.id.hex().as_bytes()).await?;
    let first = read_frame(&mut r).await?;
    if first.starts_with(b"refused") {
        bail!("{}", String::from_utf8_lossy(&first));
    }
    let their: DeviceCert = serde_json::from_slice(&first)?;
    p.check_peer(&their, conn.remote_id().as_bytes())?;
    write_frame(&mut s, &serde_json::to_vec(req)?).await?;
    // a preview may be rendered first: give it time
    let reply = tokio::time::timeout(Duration::from_secs(120), read_frame(&mut r)).await.map_err(|_| anyhow!("no answer"))??;
    conn.close(0u32.into(), b"done");
    let v: Value = serde_json::from_slice(&reply)?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        bail!("{e}");
    }
    Ok(v)
}

/// Check that `bytes` fit on the disk holding `dir` by briefly reserving them (E28; Windows
/// allocates on set_len, elsewhere this only catches some cases).
pub fn reserve(dir: &Path, bytes: u64) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join(".space-check");
    let r = std::fs::File::create(&p).and_then(|f| f.set_len(bytes));
    let _ = std::fs::remove_file(&p);
    r.map_err(|e| anyhow!("{e}"))
}

/// Fetch one blob from one teammate (E28 + E39 from the lab's `fetch-media`): what is already
/// here is not fetched again (a cut download continues), the size is proven first and must fit
/// on the disk, and `progress(done, total)` is called as bytes arrive. Returns bytes received.
async fn fetch(node: &Node, device: &str, hash: &str, mut progress: impl FnMut(u64, u64)) -> Result<u64> {
    use futures_lite::StreamExt;
    let m = node.media.get().context("media not started")?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let had = m.store.remote().local(h).await?;
    if had.is_complete() {
        return Ok(0);
    }
    let ep = node.endpoint.get().context("not started")?.clone();
    let id = iroh::EndpointId::from_bytes(&hex::decode(device)?.try_into().map_err(|_| anyhow!("bad device id"))?)?;
    let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from(id), iroh_blobs::ALPN)).await.map_err(|_| anyhow!("not reachable"))??;
    let (size, _) = iroh_blobs::get::request::get_verified_size(&conn, &h).await.map_err(|e| {
        if e.to_string().contains("ermission") {
            anyhow!("that teammate refused this device ({e})")
        } else {
            anyhow!("that teammate does not have this file ({e})")
        }
    })?;
    let need = size.saturating_sub(had.local_bytes()) + (16 << 20);
    if let Err(e) = reserve(&m.dir, need) {
        conn.close(0u32.into(), b"no space");
        bail!("not enough disk space: this needs {} MB more ({e}). Free some space and ask again; nothing already received is lost.", need / 1_000_000);
    }
    let base = had.local_bytes();
    progress(base, size);
    let mut s = std::pin::pin!(m.store.remote().fetch(conn.clone(), h).stream());
    let mut got = 0u64;
    let stats = loop {
        match s.next().await {
            Some(iroh_blobs::api::remote::GetProgressItem::Progress(n)) => {
                got = n;
                progress((base + n).min(size), size);
            }
            Some(iroh_blobs::api::remote::GetProgressItem::Done(st)) => break st,
            Some(iroh_blobs::api::remote::GetProgressItem::Error(e)) => bail!("stopped after {} MB ({e}); ask again to continue where it stopped", got / 1_000_000),
            None => bail!("stopped after {} MB; ask again to continue where it stopped", got / 1_000_000),
        }
    };
    conn.close(0u32.into(), b"done");
    progress(size, size);
    Ok(stats.payload_bytes_read)
}

/// Teammates online now, the project owner's devices first (they hold the originals).
fn sources(p: &Proj) -> Vec<String> {
    let names = p.names();
    let owner = &p.project.genesis.owner;
    let mut v: Vec<String> = p.online().into_keys().collect();
    v.sort_by_key(|d| names.get(d).and_then(|n| n.get("account")).and_then(Value::as_str) != Some(owner.as_str()));
    v
}

fn file_name_for(dir: &Path, name: &str, ext: Option<&str>, hash: &str) -> PathBuf {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| hash[..12].to_string());
    let file = match ext {
        Some(e) => format!("{stem}.{e}"),
        None => Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(stem.clone()),
    };
    let p = dir.join(&file);
    if p.exists() {
        dir.join(format!("{}-{}", &hash[..8], file))
    } else {
        p
    }
}

// ---- previews first: the loop each Light-bound project runs ------------------------------------

/// Watch the project; whenever it has photos the window does not show, bring previews in.
pub fn spawn(node: Arc<Node>, p: Arc<Proj>) {
    if p.binding() == Binding::Toy {
        return;
    }
    tokio::spawn(async move {
        let mut heads = p.heads.subscribe();
        let mut quiet = false;
        loop {
            // a new head, or every few seconds (a teammate came online, Light restarted)
            let _ = tokio::time::timeout(Duration::from_secs(if quiet { 10 } else { 3 }), heads.recv()).await;
            if p.head().is_none() || node.media.get().is_none() {
                continue;
            }
            match fill(&node, &p).await {
                Ok(n) => {
                    quiet = n == 0;
                    if quiet {
                        ahead(&node, &p).await;
                    }
                }
                Err(e) => {
                    node.emit(json!({"event": "media", "project": p.project.id.hex(), "error": format!("{e:#}")}));
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
}

/// Make the previews of this device's originals before anyone asks, one at a time, so a
/// teammate who joins gets photos in seconds. Only for projects shared with someone.
async fn ahead(node: &Arc<Node>, p: &Arc<Proj>) {
    let (Some(m), true) = (node.media.get(), p.project.genesis.members.len() > 1) else { return };
    if m.ahead_off.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Ok(originals) = originals_in(&p.store, p.head()) else { return };
    for a in originals.values() {
        if m.made.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&a.hash) {
            continue;
        }
        let mine = p.with_light(&node.device, |c| Ok(c.original_path(&a.hash).is_some())).unwrap_or(false);
        if mine {
            if let Err(e) = offer_inner(node, p, &json!({"hash": a.hash, "role": "preview"})).await {
                eprintln!("[kopanod] media: previews are not made ahead on this device: {e:#}");
                m.ahead_off.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
    }
}

/// One round: previews for every photo the window lacks. Returns how many are still missing.
async fn fill(node: &Arc<Node>, p: &Arc<Proj>) -> Result<usize> {
    let wanted = p.with_light(&node.device, |c| absent(p, c))?;
    if wanted.is_empty() {
        return Ok(0);
    }
    let m = node.media.get().context("media not started")?.clone();
    let here = local(p);
    let pid = p.project.id.hex();
    let mut ready: Vec<(String, Local)> = Vec::new();
    let mut need: Vec<(String, AssetRef)> = Vec::new();
    for (k, a) in wanted {
        match here.get(&k) {
            // the file is still here (Light restarted, or a window was linked again)
            Some(l) if Path::new(&l.path).is_file() && l.original.hash == a.hash => ready.push((k, l.clone())),
            _ => need.push((k, a)),
        }
    }
    let total = ready.len() + need.len();
    let started = Instant::now();
    let (mut bytes, mut done) = (0u64, 0usize);
    if !need.is_empty() {
        node.emit(json!({"event": "media", "project": pid, "photos_waiting": need.len(), "photos_ready": 0}));
    }
    let dir = m.preview_dir().join(&pid[..8]);
    std::fs::create_dir_all(&dir)?;
    let mut failed = 0usize;
    let mut applied = 0usize;
    // files already here (a restarted window) come back at once, without the network
    if !ready.is_empty() {
        applied += bring_in(node, p, &ready, false).await?;
    }
    for (k, a) in &need {
        let mut got = None;
        for dev in sources(p) {
            let o = match ask(node, p, &dev, &json!({"op": "media", "hash": a.hash, "role": "preview"})).await {
                Ok(v) => serde_json::from_value::<Offer>(v)?,
                Err(e) => {
                    eprintln!("[kopanod] media: {} has no preview of {}: {e:#}", &dev[..8], &k[..8.min(k.len())]);
                    continue;
                }
            };
            match fetch(node, &dev, &o.hash, |_, _| {}).await {
                Ok(n) => {
                    bytes += n;
                    got = Some(o);
                    break;
                }
                Err(e) => eprintln!("[kopanod] media: preview from {}: {e:#}", &dev[..8]),
            }
        }
        let Some(o) = got else {
            failed += 1;
            continue;
        };
        let path = file_name_for(&dir, &o.name, Some("jpg"), &o.hash);
        m.store.blobs().export(o.hash.parse::<iroh_blobs::Hash>()?, &path).await?;
        // each photo shows as soon as its preview is here: the first one in seconds, not after all
        let l = Local { kind: "preview".into(), path: path.to_string_lossy().to_string(), name: o.name.clone(), original: a.clone(), preview: Some(o.hash) };
        applied += bring_in(node, p, &[(k.clone(), l)], false).await?;
        m.refs.lock().unwrap_or_else(|e| e.into_inner()).remove(&p.project.id);
        done += 1;
        node.emit(json!({"event": "media", "project": pid, "photos_waiting": need.len() - done - failed, "photos_ready": ready.len() + done,
            "photo": o.name, "mb_done": (bytes as f64 / 1e4).round() / 100.0, "ms": started.elapsed().as_millis() as u64}));
    }
    if total > 0 {
        node.emit(json!({"event": "media", "project": pid, "photos_waiting": failed, "photos_ready": ready.len() + done, "applied": applied,
            "mb": (bytes as f64 / 1e4).round() / 100.0, "ms": started.elapsed().as_millis() as u64, "done": failed == 0}));
    }
    Ok(failed)
}

/// Import files into the window and give them the project's settings, with nothing else
/// writing meanwhile: the window's own edits are saved to the project first, and the watch
/// loop cannot save the half-imported state (it would read fresh photos as "edits").
async fn bring_in(node: &Arc<Node>, p: &Arc<Proj>, files: &[(String, Local)], original: bool) -> Result<usize> {
    let _w = p.write.lock().await;
    let saved = p.with_light(&node.device, |c| {
        let mut repo = p.repo(&node.device)?;
        let v = repo.save(&mut View::new(&p.store, p.head(), c), "Edit in Light")?;
        if v.is_some() {
            p.set_head(repo.head)?;
        }
        Ok(v)
    })?;
    if saved.is_some() {
        if let Some(h) = p.head() {
            let _ = p.heads.send(h);
        }
    }
    let before = local(p);
    let mut here = before.clone();
    for (k, l) in files {
        here.insert(k.clone(), l.clone());
    }
    save_local(p, &here)?;
    let keys: BTreeSet<String> = files.iter().map(|(k, _)| k.clone()).collect();
    let paths: Vec<String> = files.iter().map(|(_, l)| l.path.clone()).collect();
    p.with_light(&node.device, |c| {
        prepare(node.media.get(), p, c);
        // until the originals are in, the previews they replace keep their aliases (so they can
        // be found and taken out); the original wins over its preview meanwhile
        for (k, a) in aliases(&before) {
            c.previews.entry(k).or_insert(a);
        }
        let t = Instant::now();
        c.snapshot()?;
        let fresh: Vec<String> = files.iter().filter(|(k, _)| if original { !c.has_original(k) } else { !c.has_photo(k) }).map(|(_, l)| l.path.clone()).collect();
        if !fresh.is_empty() {
            c.import_files(&fresh)?;
        }
        let t_import = t.elapsed().as_millis();
        wait_shown(c, &keys, original, 60 + 5 * paths.len() as u64)?;
        let t_shown = t.elapsed().as_millis();
        let n = apply_head(p, c, &keys)?;
        eprintln!("[kopanod] media: {} file(s) in Light: import {t_import} ms, shown {t_shown} ms, settings {} ms", keys.len(), t.elapsed().as_millis());
        if original {
            // the original is in, with the project's settings: the preview stands down
            for k in &keys {
                if let Some(id) = c.alias_id(k) {
                    c.remove_photo(id)?;
                }
            }
        }
        c.previews = aliases(&here);
        Ok(n)
    })
}

// ---- original on request -------------------------------------------------------------------------

/// Start fetching one photo's original; progress arrives as `original` events.
pub fn fetch_original(node: &Arc<Node>, p: &Arc<Proj>, key: &str) -> Result<Value> {
    let key = base(key).to_string();
    let a = originals_in(&p.store, p.head())?.remove(&key).context("no such photo in the project")?;
    if p.with_light(&node.device, |c| {
        c.snapshot()?;
        Ok(c.has_original(&key))
    })? {
        return Ok(json!({"already_here": true}));
    }
    let m = node.media.get().context("media not started")?;
    {
        let mut jobs = m.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.get(&key).is_some_and(|j| j["state"] == "downloading") {
            return Ok(json!({"already_started": true}));
        }
        jobs.insert(key.clone(), json!({"state": "downloading", "mb_done": 0.0, "mb_total": a.size as f64 / 1e6}));
    }
    let mb_total = (a.size as f64 / 1e4).round() / 100.0;
    let (node, p, k2) = (node.clone(), p.clone(), key.clone());
    tokio::spawn(async move {
        let r = get_original(&node, &p, &k2, &a).await;
        let pid = p.project.id.hex();
        let ev = match &r {
            Ok(v) => json!({"event": "original", "project": pid, "key": k2, "state": "ready", "photo": v["photo"], "mb": v["mb"], "ms": v["ms"], "swap_ms": v["swap_ms"]}),
            Err(e) => json!({"event": "original", "project": pid, "key": k2, "state": "failed", "error": format!("{e:#}")}),
        };
        if let Some(m) = node.media.get() {
            m.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(k2.clone(), ev.clone());
        }
        node.emit(ev);
    });
    Ok(json!({"started": true, "mb_total": mb_total}))
}

async fn get_original(node: &Arc<Node>, p: &Arc<Proj>, key: &str, a: &AssetRef) -> Result<Value> {
    let m = node.media.get().context("media not started")?.clone();
    let pid = p.project.id.hex();
    let t = Instant::now();
    let mut last = Instant::now() - Duration::from_secs(5);
    let mut errors = Vec::new();
    let mut got = None;
    for dev in sources(p) {
        let o = match ask(node, p, &dev, &json!({"op": "media", "hash": a.hash, "role": "original"})).await {
            Ok(v) => serde_json::from_value::<Offer>(v)?,
            Err(e) => {
                errors.push(format!("{}: {e:#}", &dev[..8]));
                continue;
            }
        };
        let r = fetch(node, &dev, &a.hash, |done, total| {
            if last.elapsed() >= Duration::from_secs(1) || done == total {
                last = Instant::now();
                let ev = json!({"event": "original", "project": pid, "key": key, "photo": o.name, "state": "downloading",
                    "mb_done": (done as f64 / 1e4).round() / 100.0, "mb_total": (total as f64 / 1e4).round() / 100.0});
                m.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), ev.clone());
                node.emit(ev);
            }
        })
        .await;
        match r {
            Ok(_) => {
                got = Some(o);
                break;
            }
            Err(e) => errors.push(format!("{}: {e:#}", &dev[..8])),
        }
    }
    let Some(o) = got else { bail!("no teammate online could send the original ({})", if errors.is_empty() { "nobody online".into() } else { errors.join("; ") }) };
    let ms = t.elapsed().as_millis() as u64;
    let dir = m.dir.join("originals").join(&pid[..8]);
    std::fs::create_dir_all(&dir)?;
    let path = file_name_for(&dir, &o.name, None, &a.hash);
    // the store hands the file over instead of keeping a second copy (49 MB once, not twice)
    let opts = iroh_blobs::api::blobs::ExportOptions { hash: a.hash.parse()?, mode: iroh_blobs::api::blobs::ExportMode::TryReference, target: std::path::absolute(&path)? };
    m.store.blobs().export_with_opts(opts).await?;
    let t2 = Instant::now();
    let prev = local(p).remove(key);
    let l = Local { kind: "original".into(), path: path.to_string_lossy().to_string(), name: o.name.clone(), original: a.clone(), preview: prev.as_ref().and_then(|l| l.preview.clone()) };
    bring_in(node, p, &[(key.to_string(), l)], true).await?;
    if let Some(pv) = prev.filter(|l| l.kind == "preview") {
        let _ = std::fs::remove_file(&pv.path); // Light no longer shows it; the blob stays to hand on
    }
    Ok(json!({"photo": o.name, "mb": (a.size as f64 / 1e4).round() / 100.0, "ms": ms, "swap_ms": t2.elapsed().as_millis() as u64}))
}

/// The `media` op: where each photo of the project stands on this device.
pub fn status(node: &Arc<Node>, p: &Arc<Proj>) -> Result<Value> {
    let originals = originals_in(&p.store, p.head())?;
    let (mut from_original, mut from_preview, mut waiting) = (0, 0, Vec::new());
    if p.binding() != Binding::Toy {
        p.with_light(&node.device, |c| {
            c.snapshot()?;
            for k in originals.keys() {
                if c.has_original(k) {
                    from_original += 1;
                } else if c.has_photo(k) {
                    from_preview += 1;
                } else {
                    waiting.push(k.clone());
                }
            }
            Ok(())
        })?;
    }
    let here = local(p);
    let jobs: BTreeMap<String, Value> = node.media.get().map(|m| m.jobs.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default();
    let photos: Vec<Value> = originals
        .iter()
        .map(|(k, a)| json!({"key": k, "name": here.get(k).map(|l| l.name.clone()), "shown_from": here.get(k).map(|l| l.kind.clone()), "original_mb": (a.size as f64 / 1e4).round() / 100.0, "download": jobs.get(k)}))
        .collect();
    Ok(json!({"photos": originals.len(), "from_original": from_original, "from_preview": from_preview, "waiting": waiting.len(), "items": photos}))
}


#[cfg(test)]
mod tests {
    use super::*;
    use kopano_core::identity::{DeviceKey, Provider};
    use kopano_core::repo::Repo;
    use kopano_core::store::ObjectStore;

    fn item(kind: &str, lane: &str, state: Value, hash: Option<&str>) -> Item {
        Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets: hash.map(|h| vec![AssetRef { hash: h.into(), size: 49_000_000, role: "original".into() }]).unwrap_or_default() }
    }

    fn photo_items(key: &str, hash: &str, exposure: f64) -> Vec<(String, Item)> {
        vec![
            (format!("{key}#develop"), item("light.develop", "develop", json!({"light": {"exposure": exposure}}), Some(hash))),
            (format!("{key}#selects"), item("light.selects", "selects", json!({"rating": 0, "flag": "none", "label": null}), None)),
            (format!("{key}#meta"), item("light.meta", "meta", json!({"title": ""}), None)),
        ]
    }

    struct Team {
        project: project::Project,
        cert: DeviceCert,
        key: DeviceKey,
    }

    fn project_with(store: &RedbStore, items: Vec<(String, Item)>) -> Result<(Team, Id)> {
        let provider = Provider::generate();
        let key = DeviceKey::generate();
        let cert = provider.issue("acct-a", "a", &key, "laptop", 3_600_000)?;
        let mut members = BTreeMap::new();
        members.insert("acct-a".to_string(), project::Member { role: project::Role::Editor, lanes: vec![] });
        let pr = project::Project::create(store, "t", vec![provider.public_hex()], &cert, &key, members)?;
        let mut changes = Vec::new();
        for (k, it) in items {
            changes.push((k, Some(it.put(store)?)));
        }
        let root = tree::update(store, None, changes)?;
        let v = project::commit(store, &pr, vec![], root, "start", &cert, &key)?;
        store.flush()?;
        Ok((Team { project: pr, cert, key }, v))
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kopano-media-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A pretend Light: photos by key with three lanes each.
    struct Fake {
        photos: BTreeMap<String, Item>,
        applied: Vec<String>,
    }
    impl Connector for Fake {
        fn info(&self) -> ConnectorInfo {
            ConnectorInfo {
                app: "ubunye.light".into(),
                contract: kopano_core::connector::CONTRACT_VERSION,
                kinds: ["develop", "selects", "meta"].iter().map(|l| kopano_core::connector::KindInfo { kind: format!("light.{l}"), lane: l.to_string(), schema: 1, mergeable: true }).collect(),
            }
        }
        fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
            Ok(self.photos.clone())
        }
        fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
            if !self.shows(key) {
                bail!("Light has no photo {key}");
            }
            self.applied.push(key.into());
            match item {
                Some(i) => self.photos.insert(key.into(), i.clone()),
                None => self.photos.remove(key),
            };
            Ok(())
        }
    }
    impl Shows for Fake {
        fn shows(&self, key: &str) -> bool {
            self.photos.keys().any(|k| base(k) == base(key))
        }
    }

    #[test]
    fn originals_are_read_from_the_develop_items() -> Result<()> {
        let d = temp("orig");
        let store = RedbStore::open(d.join("k.redb"))?;
        let mut items = photo_items("e21e378d", "b3-one", 0.0);
        items.extend(photo_items("0ca370f6", "b3-two", 1.0));
        let (_, h) = project_with(&store, items)?;
        let o = originals_in(&store, Some(h))?;
        assert_eq!(o.len(), 2);
        assert_eq!(o["e21e378d"].hash, "b3-one");
        assert_eq!(originals_in(&store, None)?.len(), 0);
        drop(store);
        let _ = std::fs::remove_dir_all(&d);
        Ok(())
    }

    /// The data-loss case this module must not cause: a teammate's empty window saving
    /// "every photo removed" and sending that to the team.
    #[test]
    fn photos_still_on_their_way_are_not_saved_as_removed() -> Result<()> {
        let d = temp("save");
        let store = RedbStore::open(d.join("k.redb"))?;
        let mut items = photo_items("e21e378d", "b3-one", 1.25);
        items.extend(photo_items("0ca370f6", "b3-two", 0.0));
        let (team, h) = project_with(&store, items.clone())?;
        let mut repo = Repo { store: &store, project: team.project, head: Some(h), cert: team.cert, key: team.key };
        let mut empty = Fake { photos: BTreeMap::new(), applied: vec![] };
        assert_eq!(repo.save(&mut View::new(&store, Some(h), &mut empty), "Edit in Light")?, None, "an empty window changes nothing");
        // one photo's preview arrived and was edited: only that edit is saved
        let mut half = Fake { photos: items.iter().filter(|(k, _)| k.starts_with("e21e")).cloned().collect(), applied: vec![] };
        half.photos.get_mut("e21e378d#develop").unwrap().state = json!({"light": {"exposure": -0.5}});
        let v = repo.save(&mut View::new(&store, Some(h), &mut half), "Edit in Light")?.expect("the edit is saved");
        let now = tree::entries(&store, project::load_version(&store, &v)?.tree()?)?;
        assert_eq!(now.len(), 6, "the photo still on its way is kept");
        assert_eq!(Item::get(&store, &now["e21e378d#develop"])?.state["light"]["exposure"], json!(-0.5));
        assert_eq!(Item::get(&store, &now["0ca370f6#develop"])?.state["light"]["exposure"], json!(0.0));
        assert!(store.has(&h));
        drop(store);
        let _ = std::fs::remove_dir_all(&d);
        Ok(())
    }

    #[test]
    fn changes_for_a_photo_not_here_yet_wait_in_the_project() -> Result<()> {
        let d = temp("apply");
        let store = RedbStore::open(d.join("k.redb"))?;
        let (_, h) = project_with(&store, photo_items("e21e378d", "b3", 0.0))?;
        let mut empty = Fake { photos: BTreeMap::new(), applied: vec![] };
        let it = item("light.develop", "develop", json!({"light": {"exposure": 2.0}}), Some("b3"));
        let mut v = View::new(&store, Some(h), &mut empty);
        v.apply("e21e378d#develop", Some(&it))?;
        v.apply("e21e378d#develop", None)?;
        assert!(v.app.applied.is_empty(), "nothing is sent to a window that does not show the photo");
        drop(store);
        let _ = std::fs::remove_dir_all(&d);
        Ok(())
    }

    #[test]
    fn aliases_point_preview_files_at_the_originals_keys() {
        let a = AssetRef { hash: "b3".into(), size: 1, role: "original".into() };
        let m = BTreeMap::from([
            ("e21e378d".to_string(), Local { kind: "preview".into(), path: "T:/k/media/previews/p/DSC08379.jpg".into(), name: "DSC08379.ARW".into(), original: a.clone(), preview: Some("pv".into()) }),
            ("0ca370f6".to_string(), Local { kind: "original".into(), path: "T:/k/media/originals/p/DSC08399.ARW".into(), name: "DSC08399.ARW".into(), original: a.clone(), preview: None }),
        ]);
        let al = aliases(&m);
        assert_eq!(al.len(), 1, "an original needs no alias: Light keys it by its own content");
        let (path, alias) = al.iter().next().unwrap();
        assert_eq!(path, &norm_path("T:/k/media/previews/p/DSC08379.jpg"));
        assert_eq!(alias.key, "e21e378d");
        assert_eq!(alias.assets, vec![a]);
    }

    #[test]
    fn a_name_that_is_taken_gets_the_hash_in_front() {
        let d = temp("names");
        let p1 = file_name_for(&d, "DSC08379.ARW", Some("jpg"), "aabbccddeeff00112233");
        assert_eq!(p1.file_name().unwrap(), "DSC08379.jpg");
        std::fs::write(&p1, b"x").unwrap();
        let p2 = file_name_for(&d, "DSC08379.ARW", Some("jpg"), "aabbccddeeff00112233");
        assert_eq!(p2.file_name().unwrap(), "aabbccdd-DSC08379.jpg");
        assert_eq!(file_name_for(&d, "DSC08399.ARW", None, "ff").file_name().unwrap(), "DSC08399.ARW");
        let _ = std::fs::remove_dir_all(&d);
    }
}
