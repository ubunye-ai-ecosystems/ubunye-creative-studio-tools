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
//! 4. After every call the window gets one update (one undo step in Photo). If Photo refuses it
//!    because the user edited meanwhile, the model is dropped (nothing half-applied is sent) and
//!    `Node::integrate` saves the user's edit and tries again.
//! 5. Every tile and blob the window's items name is put in the node's media store before the
//!    version naming it is pushed, so a teammate can fetch it at once.

use crate::{media, Binding, Device, Node, Proj};
use anyhow::{anyhow, Context, Result};
use iroh::EndpointAddr;
use kopano_core::connector::{Connector, ConnectorInfo};
use kopano_core::project::{self, Item};
use kopano_core::store::{Id, ObjectStore};
use kopano_core::tree;
use kopano_lab::photo::PhotoConnector;
use std::collections::{BTreeMap, BTreeSet, HashSet};
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
}

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
        let r = f(&mut View { live: &mut *live });
        let t1 = t0.elapsed();
        let r = r.and_then(|r| settle(p, live).map(|_| r));
        if t0.elapsed() > Duration::from_millis(2000) {
            eprintln!("[kopanod] {}: Photo call took {} ms (work {} ms, update {} ms)", p.project.id.short(), t0.elapsed().as_millis(), t1.as_millis(), (t0.elapsed() - t1).as_millis());
        }
        if let Err(e) = &r {
            // never send a half-applied model (the connector also flushes when dropped)
            live.app.discard();
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
    let mut changes = Vec::new();
    for (k, item) in &have {
        let id = item.put(&p.store)?;
        if want.get(k) != Some(&id) {
            changes.push((k.clone(), Some(id)));
        }
    }
    let info = live.app.info();
    for (k, id) in &want {
        if !have.contains_key(k) && info.owns(&Item::get(&p.store, id)?.kind) {
            changes.push((k.clone(), None));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    let n = changes.len();
    let kept_root = tree::update(&p.store, root, changes)?;
    let kept = project::commit(&p.store, &p.project, vec![h], kept_root, "Kept: this window's own state when it was linked again", &dev.cert, &dev.key)?;
    p.store.flush()?;
    let mut list: Vec<String> = std::fs::read(p.dir.join("kept.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.push(kept.hex());
    kopano_core::store::write_atomic(&p.dir.join("kept.json"), &serde_json::to_vec(&list)?)?;
    let mut view = View { live: &mut *live };
    for (k, id) in &want {
        let item = Item::get(&p.store, id)?;
        if info.owns(&item.kind) && have.get(k) != Some(&item) {
            view.apply(k, Some(&item))?;
        }
    }
    for k in have.keys() {
        if !want.contains_key(k) {
            view.apply(k, None)?;
        }
    }
    eprintln!("[kopanod] {}: a Photo window was linked again; brought it to the project ({n} item(s) differed, kept as {})", p.project.id.short(), kept.short());
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
    for h in todo {
        let path = app.media_dir().join(&h);
        let r: Result<()> = rt.block_on(async {
            let hash: iroh_blobs::Hash = h.parse()?;
            if m.store.remote().local(hash).await?.is_complete() {
                return Ok(());
            }
            let opts = iroh_blobs::api::blobs::AddPathOptions { path: std::path::absolute(&path)?, format: iroh_blobs::BlobFormat::Raw, mode: iroh_blobs::api::blobs::ImportMode::TryReference };
            let tag = m.store.blobs().add_path_with_opts(opts).await?;
            anyhow::ensure!(tag.hash == hash, "media file {h} does not match its name");
            Ok(())
        });
        match r {
            Ok(()) => {
                done.insert(h);
            }
            Err(e) => eprintln!("[kopanod] media: tile {} not offered: {e:#}", &h[..8]),
        }
    }
}

const FETCH_AT_ONCE: usize = 16;

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
            let conn = match tokio::time::timeout(Duration::from_secs(5), ep.connect(EndpointAddr::from(id), iroh_blobs::ALPN)).await {
                Ok(Ok(c)) => c,
                _ => continue,
            };
            let connect_ms = tc.elapsed().as_millis();
            let tf = Instant::now();
            let asked = left.len();
            // several at once on one connection: one at a time waits on every round trip
            let mut still = Vec::new();
            let mut jobs = tokio::task::JoinSet::new();
            let mut queue = left.into_iter();
            loop {
                while jobs.len() < FETCH_AT_ONCE {
                    let Some(hash) = queue.next() else { break };
                    let (store, conn, dir) = (mem.clone(), conn.clone(), dir.to_path_buf());
                    jobs.spawn(async move {
                        let r = async {
                            store.remote().fetch(conn, hash).await.map_err(|e| anyhow!("{e}"))?;
                            // verified against its hash on the way in (BLAKE3 = the tile's name)
                            let bytes = store.blobs().get_bytes(hash).await.map_err(|e| anyhow!("{e}"))?;
                            let h = hash.to_hex().to_string();
                            let part = dir.join(format!("{h}.part"));
                            std::fs::write(&part, &bytes)?;
                            std::fs::rename(&part, dir.join(&h))?;
                            anyhow::Ok(())
                        }
                        .await;
                        (hash, r)
                    });
                }
                let Some(done) = jobs.join_next().await else { break };
                if let Ok((hash, Err(e))) = done {
                    eprintln!("[kopanod] media: tile {} from {}: {e:#}", &hash.to_hex()[..8], &dev[..8]);
                    still.push(hash);
                }
            }
            conn.close(0u32.into(), b"done");
            eprintln!("[kopanod] media: {} tile(s) from {}: connect {connect_ms} ms, transfer {} ms", asked - still.len(), &dev[..8], tf.elapsed().as_millis());
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
    use serde_json::{json, Value};

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

    #[test]
    fn removing_an_item_never_waits() -> Result<()> {
        let dir = temp("rm");
        let mut live = Live::new(Fake { items: BTreeMap::from([("layer/a".to_string(), layer(json!({})))]), dir: dir.clone(), flushed: 0 });
        View { live: &mut live }.apply("layer/a", None)?;
        assert!(live.app.items.is_empty() && live.pending.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
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
