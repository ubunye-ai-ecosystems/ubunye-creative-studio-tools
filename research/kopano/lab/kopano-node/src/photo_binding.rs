//! kopanod and Ubunye Photo: a project's items live in a running Photo window.
//!
//! The Photo connector (`kopano_lab::photo`) turns Photo's document into items: the document, one
//! item per layer, one item per 256x256 tile (the tile's pixels are an asset named by its BLAKE3
//! hash, which is also its iroh-blobs hash). This module binds such a window to a project:
//!
//! 1. Connect: `Binding::PhotoControl {port, token_file}` (the window's control port and the file
//!    holding its control token, read again on every connect because Photo writes a new one each
//!    launch) or `Binding::PhotoHeadless {cli}` (an in-memory engine with the demo document).
//! 2. A window that (re)connects is brought to the project first; it never speaks first. What it
//!    held is kept as its own version off the main line (`kept.json`, as for Light), then the
//!    project's items are applied and items the project does not have are taken out. A window
//!    that joins a project with no versions here yet stays silent until the first push, then
//!    becomes exactly the team's document (Light S3b: a fresh window's state read as "the user
//!    changed everything" wiped the team's edits).
//! 3. Tiles move as media: before the project's tile item is applied, its pixels must be in the
//!    connector's media folder. Items whose pixels are missing wait ("pending"), and they still
//!    count as the project's state, so a save never reads "tile not here yet" as "tile undone".
//!    The pixels are fetched from online teammates over iroh-blobs, through media.rs's gate
//!    (only a current member's device, only hashes this project references).
//! 4. After every call the window gets one update (one undo step in Photo). The update is built
//!    on the very snapshot the call saved first (`Repo::integrate`), so Photo refuses it when the
//!    user edited after that snapshot. Then the model is dropped (nothing half-applied is sent),
//!    what the call did to pending tiles and to a joining window is undone (`Live::rewind`), and
//!    `Node::integrate` saves the user's edit and tries again.
//! 5. Every tile and blob the window's items name is put in the node's media store before the
//!    version naming it is pushed, so a teammate can fetch it at once.
//! 6. Tiles travel packed. A tile item names its raw pixels (256 KB for 256x256 RGBA); beside
//!    each raw object the store also holds its zstd form under that form's own BLAKE3 (Photo's
//!    own bundle bytes, kept by the connector, so packing costs nothing). Before fetching, the
//!    receiver asks the teammate which packed form stands for which raw hash (`tiles-packed`, on
//!    the member-checked sync channel), fetches the packed blob (iroh-blobs checks the packed
//!    hash on the way in), unpacks it and checks the RAW hash the signed version names before
//!    the tile is used. A wrong answer can only cost one wasted fetch: the tile is then fetched
//!    raw. A teammate whose kopanod predates this refuses the question, and everything is
//!    fetched raw, as before; an older receiver never asks. The gate serves a packed blob only
//!    to a device that may fetch its raw object. Items are unchanged, so merges and old peers
//!    see the same identity. `KOPANO_PHOTO_RAW_TILES=1` turns packing off (measurements).

use crate::{media, Binding, Device, Node, Proj};
use anyhow::{anyhow, Context, Result};
use iroh::EndpointAddr;
use kopano_core::connector::{Connector, ConnectorInfo};
use kopano_core::project::{self, Item};
use kopano_core::store::{Id, ObjectStore};
use kopano_core::tree;
use kopano_lab::photo::{self as photo, PhotoConnector};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

impl Binding {
    pub fn is_photo(&self) -> bool {
        matches!(self, Binding::PhotoControl { .. } | Binding::PhotoHeadless { .. })
    }
    /// The history message for an edit made in the app itself.
    pub fn edit_message(&self) -> &'static str {
        if self.is_photo() {
            "Edit in Photo"
        } else if crate::film_binding::is_film(self) {
            "Edit in Film"
        } else {
            "Edit in Light"
        }
    }
}

/// What a Photo window is to this module (the real connector, or a fake in tests).
pub trait PhotoApp: Connector {
    fn item(&self, key: &str) -> Option<&Item>;
    fn keys(&self) -> Vec<String>;
    fn discard(&mut self);
    fn flush(&mut self) -> Result<()>;
    fn media_dir(&self) -> &Path;
    fn asset_hashes(&self) -> BTreeSet<String>;
}

impl PhotoApp for PhotoConnector {
    fn item(&self, key: &str) -> Option<&Item> {
        PhotoConnector::item(self, key)
    }
    fn keys(&self) -> Vec<String> {
        PhotoConnector::keys(self)
    }
    fn discard(&mut self) {
        PhotoConnector::discard(self)
    }
    fn flush(&mut self) -> Result<()> {
        PhotoConnector::flush(self)
    }
    fn media_dir(&self) -> &Path {
        PhotoConnector::media_dir(self)
    }
    fn asset_hashes(&self) -> BTreeSet<String> {
        PhotoConnector::asset_hashes(self)
    }
}

/// A project item waiting for its pixels.
#[derive(Clone, Debug)]
struct Pending {
    /// what the window held under this key when the change was deferred
    window: Option<Item>,
    /// what the project wants there
    want: Option<Item>,
}

/// A connected window and what this module knows about it.
pub struct Live<A: PhotoApp> {
    pub app: A,
    pending: BTreeMap<String, Pending>,
    /// joined, no version here yet: report nothing, and become exactly the team's document
    fresh: bool,
    fresh_applied: BTreeSet<String>,
    last_fetch: Option<Instant>,
}

impl<A: PhotoApp> Live<A> {
    pub fn new(app: A) -> Self {
        Self { app, pending: BTreeMap::new(), fresh: false, fresh_applied: BTreeSet::new(), last_fetch: None }
    }
    fn missing(&self, item: &Item) -> bool {
        item.assets.iter().any(|a| !self.app.media_dir().join(&a.hash).is_file())
    }
    /// Media hashes pending items need that are not in the media folder.
    fn wanted_media(&self) -> BTreeSet<String> {
        self.pending.values().filter_map(|p| p.want.as_ref()).flat_map(|i| i.assets.iter()).filter(|a| !self.app.media_dir().join(&a.hash).is_file()).map(|a| a.hash.clone()).collect()
    }
    /// Apply every pending item whose pixels are here now. Returns how many are still waiting.
    fn apply_ready(&mut self) -> Result<usize> {
        let ready: Vec<String> = self.pending.iter().filter(|(_, p)| p.want.as_ref().map_or(true, |i| !self.missing(i))).map(|(k, _)| k.clone()).collect();
        for k in ready {
            let p = self.pending.remove(&k).expect("listed above");
            self.app.apply(&k, p.want.as_ref())?;
        }
        Ok(self.pending.len())
    }
    /// What a call may change in this module's view of the window, kept so a failed call can
    /// put it back (`rewind`).
    fn mark(&self) -> Mark {
        Mark { pending: self.pending.clone(), fresh: self.fresh, fresh_applied: self.fresh_applied.clone() }
    }
    /// After a failed call (Photo refused the update): the window never got the model, so what
    /// this module believed about it must be as before the call too. Otherwise a tile applied
    /// from `pending` (its pixels had come) would be neither pending nor in the window, and the
    /// next save would read the window's old tile as "the user put it back"; and a joined window
    /// whose first update was refused would stop being fresh and speak first with its own
    /// document (the S3b class).
    fn rewind(&mut self, m: Mark) {
        self.pending = m.pending;
        self.fresh = m.fresh;
        self.fresh_applied = m.fresh_applied;
    }
    /// A joined window that received the team's document: whatever else it held goes.
    fn finish_fresh(&mut self) -> Result<()> {
        if !self.fresh || self.fresh_applied.is_empty() {
            return Ok(());
        }
        for k in self.app.keys() {
            if !self.fresh_applied.contains(&k) && !self.pending.contains_key(&k) {
                self.app.apply(&k, None)?;
            }
        }
        self.fresh = false;
        self.fresh_applied.clear();
        Ok(())
    }
}

/// `Live::mark`: this module's view of the window before a call.
struct Mark {
    pending: BTreeMap<String, Pending>,
    fresh: bool,
    fresh_applied: BTreeSet<String>,
}

/// The window as the project sees it: pending items count as the project's state.
pub struct View<'a, A: PhotoApp> {
    pub live: &'a mut Live<A>,
}

impl<A: PhotoApp> Connector for View<'_, A> {
    fn info(&self) -> ConnectorInfo {
        self.live.app.info()
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        let mut s = self.live.app.snapshot()?;
        if self.live.fresh {
            // a joined window never speaks first: its own document is not the team's
            return Ok(BTreeMap::new());
        }
        let mut gone = Vec::new();
        for (k, p) in &self.live.pending {
            if s.get(k) != p.window.as_ref() {
                // the user changed this item in the window meanwhile: their edit wins
                gone.push(k.clone());
                continue;
            }
            match &p.want {
                Some(i) => s.insert(k.clone(), i.clone()),
                None => s.remove(k),
            };
        }
        for k in gone {
            self.live.pending.remove(&k);
        }
        Ok(s)
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        if self.live.fresh {
            self.live.fresh_applied.insert(key.to_string());
        }
        if item.is_some_and(|i| self.live.missing(i)) {
            let window = self.live.pending.get(key).map(|p| p.window.clone()).unwrap_or_else(|| self.live.app.item(key).cloned());
            self.live.pending.insert(key.to_string(), Pending { window, want: item.cloned() });
            return Ok(());
        }
        self.live.pending.remove(key);
        self.live.app.apply(key, item)
    }
    fn describe(&self, before: Option<&Item>, after: &Item) -> Vec<String> {
        self.live.app.describe(before, after)
    }
}

// ---- per project state --------------------------------------------------------------------------

/// Kept in `Proj`: the connected window and what was handed to the media store.
#[derive(Default)]
pub struct Slot {
    live: std::sync::Mutex<Option<Live<PhotoConnector>>>,
    node: std::sync::OnceLock<Weak<Node>>,
    /// media hashes already in the node's media store
    published: std::sync::Mutex<HashSet<String>>,
    /// (versions already read, every media hash they name): what members may fetch
    refs: std::sync::Mutex<(HashSet<Id>, HashSet<String>)>,
    /// raw media hash -> (hash of its zstd form in the store, its size)
    packed: std::sync::Mutex<HashMap<String, (String, u64)>>,
    /// zstd form's hash -> the raw hash it stands for (the gate serves it as that raw object)
    packed_of: std::sync::Mutex<HashMap<String, String>>,
}

/// Packing turned off on this device (for before/after measurements).
fn raw_tiles_only() -> bool {
    std::env::var("KOPANO_PHOTO_RAW_TILES").is_ok_and(|v| v == "1")
}

/// Largest object a packed blob may unpack to (Photo's own limit for one object).
const MAX_OBJECT: u64 = 1 << 30;

impl Slot {
    /// Drop the connection (the project was pointed at another window).
    pub fn reset(&self) {
        let mut g = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = g.as_mut() {
            l.app.discard();
        }
        *g = None;
    }
}

fn media_dir(p: &Proj, b: &Binding) -> PathBuf {
    let custom = match b {
        Binding::PhotoControl { media_dir, .. } | Binding::PhotoHeadless { media_dir, .. } => media_dir.clone(),
        _ => None,
    };
    custom.map(PathBuf::from).unwrap_or_else(|| p.dir.join("photo-media"))
}

fn connect(p: &Proj, b: &Binding) -> Result<PhotoConnector> {
    let dir = media_dir(p, b);
    match b {
        Binding::PhotoControl { port, token_file, .. } => {
            let token = std::fs::read_to_string(token_file).with_context(|| format!("cannot read Photo's control token {token_file}"))?;
            PhotoConnector::live(*port, &token, &dir).context("Photo is not running with its control port")
        }
        Binding::PhotoHeadless { cli, .. } => PhotoConnector::headless_demo(cli, &dir),
        _ => Err(anyhow!("this project is not bound to Photo")),
    }
}

/// Called from `Node::watch_app`: remember the node (for fetching tiles from teammates).
pub fn start(node: &Arc<Node>, p: &Arc<Proj>) {
    let _ = p.photo.node.set(Arc::downgrade(node));
}

/// Run `f` against this project's Photo window (connected on first use and again after a dropped
/// connection; a window that connects is brought to the project first), then give the window
/// one update and put the media its items name in the store.
pub(crate) fn with_photo<R>(p: &Proj, dev: &Device, f: impl FnOnce(&mut dyn Connector) -> Result<R>) -> Result<R> {
    let binding = p.binding();
    tokio::task::block_in_place(|| {
        let mut g = p.photo.live.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            let mut live = Live::new(connect(p, &binding)?);
            if let Err(e) = reconcile(p, dev, &mut live).and_then(|_| settle(p, &mut live)) {
                live.app.discard();
                return Err(e);
            }
            *g = Some(live);
        }
        let live = g.as_mut().context("no Photo window")?;
        let t0 = Instant::now();
        let mark = live.mark();
        let r = f(&mut View { live: &mut *live });
        let t1 = t0.elapsed();
        let r = r.and_then(|r| settle(p, live).map(|_| r));
        if t0.elapsed() > Duration::from_millis(2000) {
            eprintln!("[kopanod] {}: Photo call took {} ms (work {} ms, update {} ms)", p.project.id.short(), t0.elapsed().as_millis(), t1.as_millis(), (t0.elapsed() - t1).as_millis());
        }
        if let Err(e) = &r {
            // never send a half-applied model (the connector also flushes when dropped), and
            // forget what the call did to the pending tiles and the fresh window
            live.app.discard();
            live.rewind(mark);
            if window_gone(e) {
                *g = None; // reconnect next time (and bring that window to the project)
            }
        }
        r
    })
}

/// The connection to the window failed (closed, restarted, not answering): only then is it
/// opened again, and a window that is opened again is brought to the project first. Any other
/// error (Photo refused an update, a race inside Photo) keeps the connection: reconnecting would
/// move the window's unsaved edits off the main line.
fn window_gone(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<std::io::Error>().is_some()) || format!("{e:#}").contains("Photo stopped answering")
}

/// After a call: pixels for pending items, a joined window finished, one update to Photo, and
/// the window's media handed to the store.
fn settle(p: &Proj, live: &mut Live<PhotoConnector>) -> Result<()> {
    if !live.pending.is_empty() {
        let want = live.wanted_media();
        // offline teammates make every try wait for its timeout: at most one try every 3 s
        if !want.is_empty() && live.last_fetch.map_or(true, |t| t.elapsed() > Duration::from_secs(3)) {
            live.last_fetch = Some(Instant::now());
            let t = Instant::now();
            match fetch_tiles(p, &want, live.app.media_dir()) {
                Ok(n) => eprintln!("[kopanod] {}: {n} of {} tile(s) fetched from teammates in {} ms", p.project.id.short(), want.len(), t.elapsed().as_millis()),
                Err(e) => eprintln!("[kopanod] {}: tiles not fetched yet: {e:#}", p.project.id.short()),
            }
        }
        live.apply_ready()?;
    }
    live.finish_fresh()?;
    live.app.flush()?;
    publish(p, &live.app);
    Ok(())
}

/// Bring a (re)connected window to the project. Its own state is kept as a version off the main
/// line first, as for Light (`Proj::reconcile`), then the project's items are applied and the
/// items the project does not have are taken out.
fn reconcile<A: PhotoApp>(p: &Proj, dev: &Device, live: &mut Live<A>) -> Result<()> {
    let Some(h) = p.head() else {
        // a window joining a project: it stays silent until the team's first push
        live.fresh = !p.peers().is_empty();
        return Ok(());
    };
    let root = project::load_version(&p.store, &h)?.tree()?;
    let want = tree::entries(&p.store, root)?;
    let have = live.app.snapshot()?;
    let mut differ = Vec::new();
    for (k, item) in &have {
        let id = item.put(&p.store)?;
        if want.get(k) != Some(&id) {
            differ.push((k.clone(), id));
        }
    }
    // S8 (as `Proj::reconcile`): a state the project never had is the person's work, made while
    // the window was not linked (strokes painted while kopanod was stopped); it stays and the next
    // save takes it. Older states (a fresh window, an untouched original) are put back.
    let mut heads = vec![h];
    if crate::approve::on(p) {
        heads.extend(crate::approve::ref_versions(p));
    }
    let seen = crate::seen_states(&p.store, &heads, differ.iter().map(|(k, _)| k))?;
    let (stale, work): (Vec<_>, Vec<_>) = differ.into_iter().partition(|(k, id)| seen.get(k).is_some_and(|s| s.contains(id)));
    let mut work: std::collections::BTreeSet<String> = work.into_iter().map(|(k, _)| k).collect();
    // A layer made in this window before any update reached it has no uid stored in the document
    // yet, so a new connection names it anew: it is the head's layer under another key, not new
    // work (the same layer item, the same tiles). It goes back under the head's key.
    let missing: Vec<&String> = want.keys().filter(|k| !have.contains_key(*k) && !k.contains('@') && k.starts_with("layer/")).collect();
    let rekeyed: Vec<String> = work
        .iter()
        .filter(|k| k.starts_with("layer/") && !k.contains('@') && !want.contains_key(*k))
        .filter(|l| {
            missing.iter().any(|m| {
                let state = |id: &Id| Item::get(&p.store, id).ok().map(|i| (i.kind, i.state));
                let tiles = |pre: &str, src: &mut dyn Iterator<Item = (String, Option<(String, Value)>)>| -> BTreeMap<String, Option<(String, Value)>> { src.filter_map(|(k, v)| k.strip_prefix(pre).map(|s| (s.to_string(), v))).collect() };
                let same_layer = have.get(*l).map(|i| (i.kind.clone(), i.state.clone())) == want.get(*m).and_then(state);
                let lt = tiles(&format!("{l}@"), &mut have.iter().map(|(k, i)| (k.clone(), Some((i.kind.clone(), i.state.clone())))));
                let mt = tiles(&format!("{m}@"), &mut want.iter().map(|(k, id)| (k.clone(), state(id))));
                same_layer && lt == mt
            })
        })
        .cloned()
        .collect();
    for l in &rekeyed {
        let pre = format!("{l}@");
        work.retain(|k| k != l && !k.starts_with(&pre));
    }
    if !rekeyed.is_empty() && work.iter().all(|k| k == "doc") {
        work.clear(); // the document's layer list named the same layers under the new keys
    }
    if !work.is_empty() {
        eprintln!("[kopanod] {}: a Photo window was linked again with {} item(s) edited while it was not linked; kept in the window as edits ({})", p.project.id.short(), work.len(), work.iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    let mut changes: Vec<(String, Option<Id>)> = stale.into_iter().map(|(k, id)| (k, Some(id))).collect();
    for (k, item) in &have {
        if rekeyed.iter().any(|l| k == l || k.starts_with(&format!("{l}@"))) || (k == "doc" && !rekeyed.is_empty() && !work.contains(k) && want.get(k) != Some(&item.put(&p.store)?)) {
            changes.push((k.clone(), Some(item.put(&p.store)?)));
        }
    }
    let info = live.app.info();
    for (k, id) in &want {
        // an item the window lacks comes back (an offline delete is undone, never a loss)
        if !have.contains_key(k) && info.owns(&Item::get(&p.store, id)?.kind) {
            changes.push((k.clone(), None));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    let n = changes.len();
    let named: Vec<String> = changes.iter().take(4).map(|(k, _)| k.clone()).collect();
    let kept_root = tree::update(&p.store, root, changes)?;
    let kept = project::commit(&p.store, &p.project, vec![h], kept_root, "Kept: this window's own state when it was linked again", &dev.cert, &dev.key)?;
    p.store.flush()?;
    let mut list: Vec<String> = std::fs::read(p.dir.join("kept.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.push(kept.hex());
    kopano_core::store::write_atomic(&p.dir.join("kept.json"), &serde_json::to_vec(&list)?)?;
    let mut view = View { live: &mut *live };
    for (k, id) in &want {
        if work.contains(k) {
            continue;
        }
        let item = Item::get(&p.store, id)?;
        if info.owns(&item.kind) && have.get(k) != Some(&item) {
            view.apply(k, Some(&item))?;
        }
    }
    for k in have.keys() {
        if !want.contains_key(k) && !work.contains(k) {
            view.apply(k, None)?;
        }
    }
    eprintln!("[kopanod] {}: a Photo window was linked again; brought it to the project ({n} item(s) held older states, kept as {}: {})", p.project.id.short(), kept.short(), named.join(", "));
    Ok(())
}

// ---- media: tiles to and from teammates ---------------------------------------------------------

/// Put every tile and blob the window's items name in the node's media store (by reference: the
/// file in the media folder is the one copy), so teammates can fetch them as soon as the version
/// naming them is pushed.
fn publish(p: &Proj, app: &PhotoConnector) {
    let Some(m) = p.media.get() else { return };
    let mut done = p.photo.published.lock().unwrap_or_else(|e| e.into_inner());
    let todo: Vec<String> = app.asset_hashes().into_iter().filter(|h| !done.contains(h)).collect();
    if todo.is_empty() {
        return;
    }
    let rt = tokio::runtime::Handle::current();
    let pack = !raw_tiles_only();
    for h in todo {
        let path = app.media_dir().join(&h);
        let zpath = photo::packed_path(app.media_dir(), &h);
        let r: Result<Option<(String, u64)>> = rt.block_on(async {
            let hash: iroh_blobs::Hash = h.parse()?;
            if !m.store.remote().local(hash).await?.is_complete() {
                let opts = iroh_blobs::api::blobs::AddPathOptions { path: std::path::absolute(&path)?, format: iroh_blobs::BlobFormat::Raw, mode: iroh_blobs::api::blobs::ImportMode::TryReference };
                let tag = m.store.blobs().add_path_with_opts(opts).await?;
                anyhow::ensure!(tag.hash == hash, "media file {h} does not match its name");
            }
            if !pack {
                return Ok(None);
            }
            if !zpath.is_file() {
                // normally kept by the connector already (Photo's own bytes); else made once here
                let raw = std::fs::read(&path)?;
                photo::keep_packed(app.media_dir(), &h, &photo::pack(&raw), raw.len())?;
                if !zpath.is_file() {
                    return Ok(None); // does not get smaller: it travels raw
                }
            }
            let size = std::fs::metadata(&zpath)?.len();
            let opts = iroh_blobs::api::blobs::AddPathOptions { path: std::path::absolute(&zpath)?, format: iroh_blobs::BlobFormat::Raw, mode: iroh_blobs::api::blobs::ImportMode::TryReference };
            let tag = m.store.blobs().add_path_with_opts(opts).await?;
            Ok(Some((tag.hash.to_hex().to_string(), size)))
        });
        match r {
            Ok(z) => {
                if let Some((zh, size)) = z {
                    p.photo.packed_of.lock().unwrap_or_else(|e| e.into_inner()).insert(zh.clone(), h.clone());
                    p.photo.packed.lock().unwrap_or_else(|e| e.into_inner()).insert(h.clone(), (zh, size));
                }
                done.insert(h);
            }
            Err(e) => eprintln!("[kopanod] media: tile {} not offered: {e:#}", &h[..8]),
        }
    }
}

/// Answer a member's `{"op": "tiles-packed", "hashes": [raw...]}`: the packed form of each raw
/// object this device holds packed and the asker may fetch. Hashes it cannot answer are left out
/// (the asker fetches those raw).
pub fn packed_offer(p: &Proj, req: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if !raw_tiles_only() {
        let asked: Vec<String> = req.get("hashes").and_then(Value::as_array).into_iter().flatten().take(1 << 16).filter_map(|h| h.as_str().map(String::from)).collect();
        let packed = p.photo.packed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for h in asked {
            if let Some((z, size)) = packed.get(&h) {
                if serves_raw(p, &h) {
                    out.insert(h, json!({"hash": z, "size": size}));
                }
            }
        }
    }
    json!({"packed": out})
}

const FETCH_AT_ONCE: usize = 16;

/// The teammate's `tiles-packed` answer: raw hash -> packed hash (well-formed entries only).
fn packed_answer(v: &Value) -> HashMap<iroh_blobs::Hash, iroh_blobs::Hash> {
    let mut out = HashMap::new();
    for (raw, z) in v.get("packed").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(r), Some(z)) = (hash64(raw), z.get("hash").and_then(Value::as_str).and_then(hash64)) {
            out.insert(r, z);
        }
    }
    out
}

/// A BLAKE3 hash from a teammate's text: 64 lowercase hex digits only (iroh's own parser panics
/// on some malformed lengths, and a teammate's answer must never stop this service).
fn hash64(s: &str) -> Option<iroh_blobs::Hash> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    s.parse().ok()
}

/// What one tile cost to bring in.
struct Got {
    /// bytes received for it (payload and verification data)
    wire: u64,
    /// its raw size
    raw: u64,
    packed: bool,
}

/// Bring one raw object into `dir`: its packed form when the teammate named one (unpacked here
/// and checked against the raw hash), else, or if that fails, the raw object itself.
async fn fetch_one(store: &iroh_blobs::store::mem::MemStore, conn: &iroh::endpoint::Connection, hash: iroh_blobs::Hash, z: Option<iroh_blobs::Hash>, dir: &Path) -> Result<Got> {
    let h = hash.to_hex().to_string();
    let write = |raw: &[u8]| -> Result<()> {
        let part = dir.join(format!("{h}.part"));
        std::fs::write(&part, raw)?;
        std::fs::rename(&part, dir.join(&h))?;
        Ok(())
    };
    let mut wire = 0;
    if let Some(z) = z {
        let r = async {
            let st = store.remote().fetch(conn.clone(), z).await.map_err(|e| anyhow!("{e}"))?;
            // the packed hash is checked on the way in; the raw hash after unpacking
            let zb = store.blobs().get_bytes(z).await.map_err(|e| anyhow!("{e}"))?;
            let h2 = h.clone();
            let (raw, zb) = tokio::task::spawn_blocking(move || photo::unpack(&zb, &h2, MAX_OBJECT).map(|raw| (raw, zb))).await??;
            anyhow::Ok((st.total_bytes_read(), raw, zb))
        }
        .await;
        match r {
            Ok((n, raw, zb)) => {
                // kept packed too, so this device can hand it on packed
                let _ = photo::keep_packed(dir, &h, &zb, raw.len());
                write(&raw)?;
                return Ok(Got { wire: n, raw: raw.len() as u64, packed: true });
            }
            Err(e) => {
                eprintln!("[kopanod] media: packed tile {} not usable, fetching it raw: {e:#}", &h[..8]);
                wire = 0;
            }
        }
    }
    let st = store.remote().fetch(conn.clone(), hash).await.map_err(|e| anyhow!("{e}"))?;
    // verified against its hash on the way in (BLAKE3 = the tile's name)
    let bytes = store.blobs().get_bytes(hash).await.map_err(|e| anyhow!("{e}"))?;
    write(&bytes)?;
    Ok(Got { wire: wire + st.total_bytes_read(), raw: bytes.len() as u64, packed: false })
}

/// Fetch these tiles from online teammates (owner's devices first) into the store, then into the
/// connector's media folder. One connection per teammate for all of them. Returns how many are
/// in the folder now.
fn fetch_tiles(p: &Proj, want: &BTreeSet<String>, dir: &Path) -> Result<usize> {
    let node = p.photo.node.get().and_then(|w| w.upgrade()).context("the service has not started")?;
    let m = node.media.get().context("media not started")?.clone();
    let ep = node.endpoint.get().context("not started")?.clone();
    let rt = tokio::runtime::Handle::current();
    std::fs::create_dir_all(dir)?;
    // Tiles arrive in a memory store and go straight to the media folder: the node's file store
    // syncs the disk once per blob, which on this PC's hard disk cost ~200 ms per 256 KB tile.
    let mem = iroh_blobs::store::mem::MemStore::new();
    let n = rt.block_on(async {
        let mut left: Vec<iroh_blobs::Hash> = Vec::new();
        for h in want {
            if !dir.join(h).is_file() {
                left.push(h.parse()?);
            }
        }
        for dev in media::sources(p) {
            if left.is_empty() {
                break;
            }
            let Ok(id) = hex::decode(&dev).map_err(|e| anyhow!("{e}")).and_then(|b| iroh::EndpointId::from_bytes(&b.try_into().map_err(|_| anyhow!("bad device id"))?).map_err(|e| anyhow!("{e}"))) else { continue };
            let tc = Instant::now();
            // which packed form stands for which tile, asked while the blobs connection opens
            let ask_packed = async {
                if raw_tiles_only() {
                    return HashMap::new();
                }
                let req = json!({"op": "tiles-packed", "hashes": left.iter().map(|h| h.to_hex().to_string()).collect::<Vec<_>>()});
                match tokio::time::timeout(Duration::from_secs(5), media::ask(&node, p, &dev, &req)).await {
                    Ok(Ok(v)) => packed_answer(&v),
                    _ => HashMap::new(), // an older kopanod: raw
                }
            };
            let (conn, packed) = tokio::join!(tokio::time::timeout(Duration::from_secs(5), ep.connect(EndpointAddr::from(id), iroh_blobs::ALPN)), ask_packed);
            let conn = match conn {
                Ok(Ok(c)) => c,
                _ => continue,
            };
            let connect_ms = tc.elapsed().as_millis();
            let tf = Instant::now();
            let asked = left.len();
            let (mut bytes, mut raw_bytes, mut n_packed) = (0u64, 0u64, 0usize);
            // several at once on one connection: one at a time waits on every round trip
            let mut still = Vec::new();
            let mut jobs = tokio::task::JoinSet::new();
            let mut queue = left.into_iter();
            loop {
                while jobs.len() < FETCH_AT_ONCE {
                    let Some(hash) = queue.next() else { break };
                    let (store, conn, dir) = (mem.clone(), conn.clone(), dir.to_path_buf());
                    let z = packed.get(&hash).copied();
                    jobs.spawn(async move {
                        let r = fetch_one(&store, &conn, hash, z, &dir).await;
                        (hash, r)
                    });
                }
                let Some(done) = jobs.join_next().await else { break };
                match done {
                    Ok((hash, Err(e))) => {
                        eprintln!("[kopanod] media: tile {} from {}: {e:#}", &hash.to_hex()[..8], &dev[..8]);
                        still.push(hash);
                    }
                    Ok((_, Ok(got))) => {
                        bytes += got.wire;
                        raw_bytes += got.raw;
                        n_packed += got.packed as usize;
                    }
                    Err(_) => {}
                }
            }
            conn.close(0u32.into(), b"done");
            let ms = tf.elapsed().as_millis() as u64;
            eprintln!(
                "[kopanod] media: {} tile(s) from {} ({n_packed} packed): {} KB moved for {} KB of pixels, connect {connect_ms} ms, transfer {ms} ms",
                asked - still.len(),
                &dev[..8],
                bytes / 1000,
                raw_bytes / 1000
            );
            node.emit(json!({"event": "tiles", "project": p.project.id.hex(), "from": &dev[..8], "tiles": asked - still.len(), "packed": n_packed, "kb": bytes / 1000, "raw_kb": raw_bytes / 1000, "ms": ms}));
            left = still;
        }
        anyhow::Ok(want.iter().filter(|h| dir.join(h.as_str()).is_file()).count())
    })?;
    let _ = m;
    Ok(n)
}

/// Every media hash this project's versions name (read once per version): what its members may
/// fetch from this device. Used by media.rs's gate.
pub fn serves(p: &Proj, hash: &str) -> bool {
    if !p.binding().is_photo() {
        return false;
    }
    // a packed form is served exactly when its raw object is
    let raw = p.photo.packed_of.lock().unwrap_or_else(|e| e.into_inner()).get(hash).cloned();
    serves_raw(p, raw.as_deref().unwrap_or(hash))
}

fn serves_raw(p: &Proj, hash: &str) -> bool {
    if !p.binding().is_photo() {
        return false;
    }
    let Some(h) = p.head() else { return false };
    let mut g = p.photo.refs.lock().unwrap_or_else(|e| e.into_inner());
    if !g.0.contains(&h) {
        if let Ok(hist) = project::history(&p.store, &[h]) {
            for (id, v) in hist {
                if g.0.contains(&id) {
                    continue;
                }
                if let Ok(entries) = v.tree().and_then(|t| tree::entries(&p.store, t)) {
                    for iid in entries.values() {
                        if let Ok(item) = Item::get(&p.store, iid) {
                            g.1.extend(item.assets.into_iter().map(|a| a.hash));
                        }
                    }
                }
                g.0.insert(id);
            }
        }
    }
    g.1.contains(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kopano_core::connector::{KindInfo, CONTRACT_VERSION};
    use kopano_core::project::AssetRef;

    /// A pretend Photo window: items by key; media in a folder; `refuse` makes the next flush fail.
    struct Fake {
        items: BTreeMap<String, Item>,
        dir: PathBuf,
        flushed: usize,
    }
    impl Connector for Fake {
        fn info(&self) -> ConnectorInfo {
            ConnectorInfo { app: "ubunye.photo".into(), contract: CONTRACT_VERSION, kinds: vec![KindInfo { kind: "photo.layer".into(), lane: "layers".into(), schema: 1, mergeable: true }, KindInfo { kind: "photo.tile.retouch".into(), lane: "retouch".into(), schema: 1, mergeable: false }] }
        }
        fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
            Ok(self.items.clone())
        }
        fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
            for a in item.into_iter().flat_map(|i| &i.assets) {
                anyhow::ensure!(self.dir.join(&a.hash).is_file(), "media {} is not here yet", a.hash);
            }
            match item {
                Some(i) => self.items.insert(key.into(), i.clone()),
                None => self.items.remove(key),
            };
            Ok(())
        }
    }
    impl PhotoApp for Fake {
        fn item(&self, key: &str) -> Option<&Item> {
            self.items.get(key)
        }
        fn keys(&self) -> Vec<String> {
            self.items.keys().cloned().collect()
        }
        fn discard(&mut self) {}
        fn flush(&mut self) -> Result<()> {
            self.flushed += 1;
            Ok(())
        }
        fn media_dir(&self) -> &Path {
            &self.dir
        }
        fn asset_hashes(&self) -> BTreeSet<String> {
            self.items.values().flat_map(|i| i.assets.iter().map(|a| a.hash.clone())).collect()
        }
    }

    fn tile(hash: &str) -> Item {
        Item { kind: "photo.tile.retouch".into(), lane: "retouch".into(), schema: 1, state: json!({"hash": hash}), assets: vec![AssetRef { hash: hash.into(), size: 4, role: "tile".into() }] }
    }
    fn layer(v: Value) -> Item {
        Item { kind: "photo.layer".into(), lane: "layers".into(), schema: 1, state: v, assets: vec![] }
    }
    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kopano-photo-binding-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tile whose pixels are not here yet waits, and still counts as the project's state: a save
    /// must not read "not here yet" as "the user put the old tile back".
    #[test]
    fn a_tile_waiting_for_its_pixels_is_not_saved_as_undone() -> Result<()> {
        let dir = temp("wait");
        std::fs::write(dir.join("old"), b"old!")?;
        let mut live = Live::new(Fake { items: BTreeMap::from([("l@s@0_0".to_string(), tile("old"))]), dir: dir.clone(), flushed: 0 });
        let mut v = View { live: &mut live };
        v.apply("l@s@0_0", Some(&tile("new")))?;
        assert_eq!(v.live.app.items["l@s@0_0"], tile("old"), "the window keeps the old tile until the pixels come");
        assert_eq!(v.snapshot()?["l@s@0_0"], tile("new"), "the project's tile is what a save sees");
        // the pixels arrive: the tile is applied and nothing waits any more
        std::fs::write(dir.join("new"), b"new!")?;
        assert_eq!(live.apply_ready()?, 0);
        assert_eq!(live.app.items["l@s@0_0"], tile("new"));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// If the user repaints a tile while the team's version of it waits for pixels, the user's
    /// stroke wins (it is the later edit) and the waiting change is dropped.
    #[test]
    fn a_user_edit_on_a_waiting_tile_wins() -> Result<()> {
        let dir = temp("user");
        for h in ["old", "mine"] {
            std::fs::write(dir.join(h), h)?;
        }
        let mut live = Live::new(Fake { items: BTreeMap::from([("l@s@0_0".to_string(), tile("old"))]), dir: dir.clone(), flushed: 0 });
        View { live: &mut live }.apply("l@s@0_0", Some(&tile("team")))?;
        live.app.items.insert("l@s@0_0".into(), tile("mine"));
        assert_eq!(View { live: &mut live }.snapshot()?["l@s@0_0"], tile("mine"));
        assert!(live.pending.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// A window joining a project says nothing about its own document, and once the team's items
    /// are in, whatever else it held is taken out (it does not leak into the project).
    #[test]
    fn a_joining_window_becomes_the_teams_document() -> Result<()> {
        let dir = temp("fresh");
        let mut live = Live::new(Fake { items: BTreeMap::from([("layer/blank".to_string(), layer(json!({"name": "Background"})))]), dir: dir.clone(), flushed: 0 });
        live.fresh = true;
        assert!(View { live: &mut live }.snapshot()?.is_empty(), "a joining window never speaks first");
        View { live: &mut live }.apply("layer/team", Some(&layer(json!({"name": "Retouch"}))))?;
        live.finish_fresh()?;
        assert_eq!(live.app.keys(), vec!["layer/team".to_string()]);
        assert!(!live.fresh);
        assert_eq!(View { live: &mut live }.snapshot()?.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Photo refused the update a call built: what the call did to the pending tiles and to a
    /// joining window is undone too. Without that, a tile applied from pending was lost from
    /// both (the next save read the window's old tile as the user's), and a joining window whose
    /// first update was refused spoke first with its own document.
    #[test]
    fn a_refused_call_leaves_pending_tiles_and_a_joining_window_as_they_were() -> Result<()> {
        let dir = temp("rewind");
        std::fs::write(dir.join("old"), b"old!")?;
        let mut live = Live::new(Fake { items: BTreeMap::from([("l@s@0_0".to_string(), tile("old"))]), dir: dir.clone(), flushed: 0 });
        View { live: &mut live }.apply("l@s@0_0", Some(&tile("new")))?;
        live.fresh = true;
        // the call: the pixels came, the pending tile is applied, the joining window finished...
        let mark = live.mark();
        std::fs::write(dir.join("new"), b"new!")?;
        View { live: &mut live }.apply("layer/team", Some(&layer(json!({"name": "Retouch"}))))?;
        live.apply_ready()?;
        live.finish_fresh()?;
        // ...then Photo refused the update: the window still holds what it held
        live.app.items = BTreeMap::from([("l@s@0_0".to_string(), tile("old"))]);
        live.rewind(mark);
        assert!(live.fresh, "still joining: it must not speak first");
        assert!(View { live: &mut live }.snapshot()?.is_empty());
        live.fresh = false;
        assert_eq!(View { live: &mut live }.snapshot()?["l@s@0_0"], tile("new"), "the team's tile still waits to be applied");
        assert_eq!(live.apply_ready()?, 0);
        assert_eq!(live.app.items["l@s@0_0"], tile("new"));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn removing_an_item_never_waits() -> Result<()> {
        let dir = temp("rm");
        let mut live = Live::new(Fake { items: BTreeMap::from([("layer/a".to_string(), layer(json!({})))]), dir: dir.clone(), flushed: 0 });
        View { live: &mut live }.apply("layer/a", None)?;
        assert!(live.app.items.is_empty() && live.pending.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Only well-formed entries of a teammate's packed answer are used; anything else is fetched raw.
    #[test]
    fn a_packed_answer_is_read_defensively() {
        let (r, z) = (blake3::hash(b"raw").to_hex().to_string(), blake3::hash(b"zst").to_hex().to_string());
        let v = json!({"packed": {r.clone(): {"hash": z.clone(), "size": 3}, "not-a-hash": {"hash": z.clone()}, z.clone(): {"hash": 7}}});
        let m = packed_answer(&v);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&r.parse::<iroh_blobs::Hash>().unwrap()].to_hex().to_string(), z);
        assert!(packed_answer(&json!({"error": "unknown request"})).is_empty());
    }

    #[test]
    fn bindings_read_from_the_attach_request() {
        let b: Binding = serde_json::from_value(json!({"kind": "photo-control", "port": 18301, "token_file": "T:/x/a.token"})).unwrap();
        assert_eq!(b, Binding::PhotoControl { port: 18301, token_file: "T:/x/a.token".into(), media_dir: None });
        assert!(b.is_photo());
        assert_eq!(b.edit_message(), "Edit in Photo");
        assert!(!Binding::LightControl { port: 1 }.is_photo());
    }
}
