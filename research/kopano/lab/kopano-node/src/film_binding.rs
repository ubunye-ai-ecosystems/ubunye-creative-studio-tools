//! kopanod and Ubunye Film: a project that lives in a Film window (Phase 5, after E40).
//!
//! The Film connector (`kopano-lab/src/film.rs`) turns Film's timeline into items (per clip an
//! edit, a grade and a sound part; per track an edit and a mix part). This module is what
//! kopanod adds around it, so two people can cut one film from two computers:
//!
//! 1. **Id blocks.** Every id Film makes comes from a block (`block * 2^24 + n`, Film branch
//!    `feature/kopano-stable-ids`). kopanod gives each window it is linked to a block derived
//!    from this device, the project and a counter (`kopano.idBlock {block}`); Film refuses a block
//!    that any id of the project already uses, and kopanod then takes the next one. Two people
//!    therefore never make the same clip id. A window that opens another file (or a new one)
//!    moves to a block of its own; kopanod notices the change and treats the window as linked
//!    again (point 2).
//! 2. **A window never speaks first.** A window kopanod (re)connects to, or one that reopened a
//!    file, is brought to the project's head before anything it holds is saved: its own state is
//!    kept as a version off the main line (`kept.json`), then every item of the head is applied
//!    and every clip the head does not have is taken out. This is the Light S3b data-loss class
//!    (a fresh window read as "the user reset everything" and pushed to the team), closed for
//!    Film too, including clips the team removed coming back from an old file.
//!    A device that joined and has no head yet shows nothing to the project until the first
//!    push has landed (so its copy of the project file cannot start a second history).
//! 3. **Footage that is not here yet is not a change.** A clip whose original this window does
//!    not have counts as the project's version of that clip (`View::snapshot`), and an update the
//!    window refuses (Film said no, or the footage is still on its way) waits in `deferred` and
//!    is tried again every few seconds and whenever footage arrives. The window never sends a
//!    half-applied or refused state back to the team.
//! 4. **Footage moves through media.rs**, membership-checked: first a small proxy (Film's own
//!    `media.createProxies`, H.264 at a quarter size, made by the device that has the original,
//!    with a headless Film engine so the owner's window is not touched), attached to the
//!    project's media item so the clip plays; the original on request (`fetch-original`), then
//!    the media item is relinked to it, so exports are made from the original.
//! 5. **Flush.** After team changes are brought into a window, kopanod saves Film's project file
//!    (`file.save`, only when the window has one), so a crash or a reopen does not show old work.
//!
//! Per project: `film.json` (counter for id blocks, media names and files here).

use crate::media::{self, Offer};
use crate::{Binding, Device, Node, Proj};
use anyhow::{bail, Context, Result};
use kopano_core::connector::{Connector, ConnectorInfo};
use kopano_core::identity::{DeviceCert, DeviceKey};
use kopano_core::project::{self, Item, Project};
use kopano_core::store::{Id, ObjectStore, RedbStore};
use kopano_core::tree;
use kopano_lab::film::{FilmConnector, Fresh, MediaHere, Transport};
use kopano_lab::light::norm_path;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Film's id blocks: 1 to 2^29 - 1 (ids stay below 2^53).
pub const MAX_ID_BLOCK: u64 = (1 << 29) - 1;
/// Proxy preset handed to Film: H.264 at a quarter of the frame size.
pub const PROXY_PRESET: &str = "h264_quarter";

pub fn is_film(b: &Binding) -> bool {
    matches!(b, Binding::FilmControl { .. } | Binding::FilmHeadless { .. })
}

// ---- what kopanod keeps per project and per window ----------------------------------------------

/// One original of the project as this device knows it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FilmMedia {
    pub name: String,
    pub size: u64,
    /// proxy file here (received from a teammate)
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub proxy_hash: Option<String>,
    /// original file here (received from a teammate)
    #[serde(default)]
    pub original: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State {
    /// counter behind the id blocks this device hands out for this project
    #[serde(default)]
    pub block_n: u64,
    /// original's BLAKE3 -> what this device knows about it
    #[serde(default)]
    pub media: BTreeMap<String, FilmMedia>,
}

pub fn state(p: &Proj) -> State {
    std::fs::read(p.dir.join("film.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn save_state(p: &Proj, s: &State) -> Result<()> {
    kopano_core::store::write_atomic(&p.dir.join("film.json"), &serde_json::to_vec_pretty(s)?)
}

/// A Film window kopanod is linked to.
pub struct Slot {
    pub conn: FilmConnector,
    /// items the window refused or could not take yet
    pub deferred: BTreeMap<String, Deferred>,
    /// the id block this window was given
    pub block: Option<u64>,
    /// not brought to the project yet
    pub fresh: bool,
    /// proxies already attached in this window (media item ids)
    pub attached: BTreeSet<u64>,
    pub proxies_on: bool,
    /// "waiting for this project" was said for this window
    pub waiting_said: bool,
    /// the window has a project file to save to (Film's desktop app only)
    pub control: bool,
}

/// An update the window refused or could not take yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Deferred {
    /// what the window held under this key when the update was refused
    pub window: Option<Item>,
    /// what the project wants there
    pub want: Option<Item>,
}

/// The id block `n` of this device for this project.
pub fn derive_block(device: &str, project: &Id, n: u64) -> u64 {
    let h = blake3::hash(format!("film-id-block:{device}:{}:{n}", project.hex()).as_bytes());
    let x = u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap_or_default());
    1 + x % MAX_ID_BLOCK
}

fn current_block(c: &mut FilmConnector) -> Result<u64> {
    c.t.exec("kopano.idBlock", json!({}))?.get("block").and_then(Value::as_u64).context("this Film build cannot share a project yet (no id blocks): update Film")
}

/// Give the window an id block of its own: derived from this device, refused by Film when an
/// id of the project already lives in it (then the next one).
fn hand_block(p: &Proj, dev: &Device, c: &mut FilmConnector) -> Result<u64> {
    let mut st = state(p);
    for _ in 0..64 {
        let b = derive_block(&dev.id(), &p.project.id, st.block_n);
        match c.t.exec("kopano.idBlock", json!({"block": b})) {
            Ok(_) => {
                save_state(p, &st)?;
                return Ok(b);
            }
            Err(e) if format!("{e:#}").contains("already used") => st.block_n += 1,
            Err(e) => return Err(e.context("this Film build cannot share a project yet (no id blocks): update Film")),
        }
    }
    bail!("no free id block for this window after 64 tries")
}

/// What the connector needs to know about this project's media on this device.
fn prepare(p: &Proj, c: &mut FilmConnector) {
    for (h, m) in state(p).media {
        if !m.name.is_empty() {
            c.offline.insert((m.name.clone(), m.size), h.clone());
        }
        if let Some(pr) = m.proxy.as_ref().filter(|f| Path::new(f).is_file()) {
            c.aliases.insert(norm_path(pr), (h.clone(), m.size));
            c.media_files.entry(h.clone()).or_insert_with(|| pr.clone());
        }
        if let Some(o) = m.original.as_ref().filter(|f| Path::new(f).is_file()) {
            c.media_files.insert(h.clone(), o.clone());
        }
    }
}

/// Run `f` against the window: connected on first use and after a dropped connection; a window
/// that (re)connects or reopened a file is brought to the project first and given its id block.
pub fn with_slot<R>(p: &Proj, dev: &Device, f: impl FnOnce(&mut Slot, Option<Id>, bool) -> Result<R>) -> Result<R> {
    let binding = p.binding();
    tokio::task::block_in_place(|| {
        let mut g = p.film.lock().unwrap_or_else(|e| e.into_inner());
        let r = (|| {
            if g.is_none() {
                let (conn, control) = match &binding {
                    Binding::FilmControl { port } => (FilmConnector::live(*port, Fresh::Windows { ports: vec![*port], project: String::new() }).context("Film is not running with its control port")?, true),
                    Binding::FilmHeadless { cli, project } => (FilmConnector::headless(cli, project.as_deref())?, false),
                    _ => bail!("this project is not bound to Film"),
                };
                *g = Some(Slot { conn, deferred: BTreeMap::new(), block: None, fresh: true, attached: BTreeSet::new(), proxies_on: false, waiting_said: false, control });
            }
            let slot = g.as_mut().context("no window")?;
            prepare(p, &mut slot.conn);
            // a window that opened another file (or a new one) moved to a block of its own
            if let Some(b) = slot.block {
                let now = current_block(&mut slot.conn)?;
                if now != b {
                    eprintln!("[kopanod] {}: the Film window opened another project file; bringing it to the project again", p.project.id.short());
                    slot.fresh = true;
                    slot.block = None;
                    slot.deferred.clear();
                    slot.attached.clear();
                }
            }
            let head = p.head();
            // a device that joined takes the project from teammates before its window counts;
            // so does a window that shows no sequence of this project (Film just started with
            // nothing open, or another project is open): it is not touched until it does
            let elsewhere = slot.fresh && head.is_some() && !shows_project(p, head, &mut slot.conn)?;
            if elsewhere && !slot.waiting_said {
                eprintln!("[kopanod] {}: the Film window has another project (or none) open; waiting for this one", p.project.id.short());
                slot.waiting_said = true;
            }
            let joining = (head.is_none() && !p.peers().is_empty()) || elsewhere;
            if slot.fresh && !joining {
                // the block first, then the project: a window that opens another file meanwhile
                // shows another block, and every snapshot below checks it
                let block = hand_block(p, dev, &mut slot.conn)?;
                slot.block = Some(block);
                // the whole head is applied below: earlier waiting updates are part of it
                slot.deferred.clear();
                if let Some(h) = head {
                    let mut v = View::new(&p.store, Some(h), &mut slot.conn, &mut slot.deferred);
                    v.expect_block = Some(block);
                    let (kept, n) = bring_to_head(&p.store, &p.project, &dev.cert, &dev.key, h, &mut v)?;
                    if let Some(k) = kept {
                        let mut list: Vec<String> = std::fs::read(p.dir.join("kept.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
                        list.push(k.hex());
                        kopano_core::store::write_atomic(&p.dir.join("kept.json"), &serde_json::to_vec(&list)?)?;
                        eprintln!("[kopanod] {}: a Film window was linked again; brought it to the project ({n} item(s) differed, its own state kept as {})", p.project.id.short(), k.short());
                    }
                    if n > 0 && slot.control {
                        save_file(&mut slot.conn);
                    }
                }
                if current_block(&mut slot.conn)? != block {
                    bail!("the Film window opened another project file while it was being linked");
                }
                slot.fresh = false;
            }
            f(slot, head, joining)
        })();
        if r.is_err() {
            *g = None; // reconnect (and bring the window to the project) next time
        }
        r
    })
}

/// Does the window show at least one sequence of the project's head?
fn shows_project(p: &Proj, head: Option<Id>, c: &mut FilmConnector) -> Result<bool> {
    let Some(h) = head else { return Ok(true) };
    let wanted: BTreeSet<String> = tree::entries(&p.store, project::load_version(&p.store, &h)?.tree()?)?.into_keys().filter_map(|k| k.strip_prefix("film/").and_then(|r| r.split('/').next()).map(String::from)).collect();
    if wanted.is_empty() {
        return Ok(true);
    }
    let all = c.t.exec("kopano.items", json!({}))?;
    Ok(all.get("sequences").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.get("id").and_then(Value::as_u64)).any(|id| wanted.contains(&id.to_string())))
}

/// `Proj::with_app` for Film: the window as the project sees it (`View`).
pub fn with_app<R>(p: &Proj, dev: &Device, f: impl FnOnce(&mut dyn Connector) -> Result<R>) -> Result<R> {
    with_slot(p, dev, |slot, head, joining| {
        let block = slot.block;
        let mut v = View::new(&p.store, head, &mut slot.conn, &mut slot.deferred);
        v.joining = joining;
        v.expect_block = block;
        let r = f(&mut v)?;
        let applied = v.applied;
        if applied > 0 && slot.control {
            save_file(&mut slot.conn);
        }
        Ok(r)
    })
}

/// Flush: Film keeps team changes in its project file too (only a window that has one).
fn save_file(c: &mut FilmConnector) {
    if let Err(e) = c.t.exec("file.save", json!({})) {
        let e = format!("{e:#}");
        if !e.contains("no path") {
            eprintln!("[kopanod] Film did not save its project file: {e}");
        }
    }
}

// ---- the window as the project sees it ------------------------------------------------------------

/// A Film window that can say whether it has a media item for an original.
pub trait FilmApp: Connector {
    fn has_media(&self, hash: &str) -> bool;
    /// The id block of the window's project at the last snapshot (None: not known).
    fn block(&self) -> Option<u64> {
        None
    }
}
impl FilmApp for FilmConnector {
    fn has_media(&self, hash: &str) -> bool {
        FilmConnector::has_media(self, hash)
    }
    fn block(&self) -> Option<u64> {
        self.last_block
    }
}

/// `film/S/clip/C#part` -> `film/S/clip/C`
fn base(k: &str) -> &str {
    k.rsplit_once('#').map(|(b, _)| b).unwrap_or(k)
}

/// Film as the project sees it:
/// - a device that is joining shows nothing until the project's first version is here;
/// - a clip whose original has no media item in this window yet is the project's clip (all
///   its parts), not "removed" or "changed";
/// - an update the window refused is the project's item until the window takes it.
pub struct View<'a, A: FilmApp + ?Sized> {
    store: &'a RedbStore,
    head: Option<Id>,
    pub app: &'a mut A,
    pub deferred: &'a mut BTreeMap<String, Deferred>,
    pub joining: bool,
    /// the block this window was given: a snapshot from another block is another project file
    pub expect_block: Option<u64>,
    /// updates the window took
    pub applied: usize,
    /// the window's own items at the last snapshot (what a refused update leaves there)
    seen: Option<BTreeMap<String, Item>>,
}

impl<'a, A: FilmApp + ?Sized> View<'a, A> {
    pub fn new(store: &'a RedbStore, head: Option<Id>, app: &'a mut A, deferred: &'a mut BTreeMap<String, Deferred>) -> Self {
        Self { store, head, app, deferred, joining: false, expect_block: None, applied: 0, seen: None }
    }
    /// Try every waiting update again. Returns how many the window took. Called right after a
    /// save (which kept any edit the user made to a waiting item, see `snapshot`), so what the
    /// window shows now is the new reference for an update it refuses again: footage that came
    /// meanwhile changes how a clip reads, and that is not an edit by the user.
    pub fn retry(&mut self) -> usize {
        let before = self.applied;
        let Ok(now) = self.app.snapshot() else { return 0 };
        for (k, d) in self.deferred.clone() {
            let _ = self.apply(&k, d.want.as_ref());
            if let Some(still) = self.deferred.get_mut(&k) {
                still.window = now.get(&k).cloned();
            }
        }
        self.applied - before
    }
}

fn media_of(item: &Item) -> Option<&str> {
    if item.kind == "film.clip" {
        item.state.get("media").and_then(Value::as_str)
    } else {
        None
    }
}

impl<A: FilmApp + ?Sized> Connector for View<'_, A> {
    fn info(&self) -> ConnectorInfo {
        self.app.info()
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        if self.joining {
            // no opinion yet: the project's own state (never "nothing", which reads as "the
            // user removed every clip": S6 lost a whole timeline that way to an empty window)
            let Some(h) = self.head else { return Ok(BTreeMap::new()) };
            let info = self.app.info();
            let mut s = BTreeMap::new();
            for (k, id) in tree::entries(self.store, project::load_version(self.store, &h)?.tree()?)? {
                let it = Item::get(self.store, &id)?;
                if info.owns(&it.kind) {
                    s.insert(k, it);
                }
            }
            return Ok(s);
        }
        let raw = self.app.snapshot()?;
        let mut s = raw.clone();
        if let (Some(want), Some(now)) = (self.expect_block, self.app.block()) {
            if want != now {
                bail!("the Film window opened another project file; it is linked again before anything it shows is saved");
            }
        }
        if let Some(h) = self.head {
            let info = self.app.info();
            let want: Vec<(String, Item)> = tree::entries(self.store, project::load_version(self.store, &h)?.tree()?)?
                .into_iter()
                .map(|(k, id)| Ok((k, Item::get(self.store, &id)?)))
                .collect::<Result<_>>()?;
            let waiting: BTreeSet<String> = want.iter().filter(|(_, it)| media_of(it).is_some_and(|m| !self.app.has_media(m))).map(|(k, _)| base(k).to_string()).collect();
            if !waiting.is_empty() {
                s.retain(|k, _| !waiting.contains(base(k)));
                for (k, it) in want {
                    if waiting.contains(base(&k)) && info.owns(&it.kind) {
                        s.insert(k, it);
                    }
                }
            }
        }
        // A waiting update is the project's item, until the user edits that item in the window
        // themselves: their edit is the later one and wins (as for a Photo tile waiting for its
        // pixels). Without this the edit was hidden behind the waiting update and then undone by
        // the next retry, never saved.
        self.deferred.retain(|k, d| raw.get(k) == d.window.as_ref());
        for (k, d) in self.deferred.iter() {
            match &d.want {
                Some(it) => s.insert(k.clone(), it.clone()),
                None => s.remove(k),
            };
        }
        self.seen = Some(raw);
        Ok(s)
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        match self.app.apply(key, item) {
            Ok(()) => {
                self.deferred.remove(key);
                self.applied += 1;
            }
            Err(e) => {
                let window = match self.deferred.get(key) {
                    Some(d) => d.window.clone(),
                    None => match &self.seen {
                        Some(s) => s.get(key).cloned(),
                        None => self.app.snapshot()?.get(key).cloned(),
                    },
                };
                if self.deferred.insert(key.to_string(), Deferred { window, want: item.cloned() }).is_none() {
                    eprintln!("[kopanod] Film did not take {key} yet ({e:#}); it waits and is tried again");
                }
            }
        }
        Ok(())
    }
    fn describe(&self, before: Option<&Item>, after: &Item) -> Vec<String> {
        self.app.describe(before, after)
    }
}

/// Bring a window to the project's head `h`. Whatever the window held that differs is kept as
/// a version off the main line (returned, for `kept.json`), then the head is applied and clips
/// the head does not have are taken out. Returns (kept version, items that differed).
pub fn bring_to_head<A: FilmApp + ?Sized>(store: &RedbStore, pr: &Project, cert: &DeviceCert, key: &DeviceKey, h: Id, app: &mut View<'_, A>) -> Result<(Option<Id>, usize)> {
    let root = project::load_version(store, &h)?.tree()?;
    let want = tree::entries(store, root)?;
    let have = app.snapshot()?;
    let info = app.info();
    let mut changes = Vec::new();
    for (k, item) in &have {
        let id = item.put(store)?;
        if want.get(k) != Some(&id) {
            changes.push((k.clone(), Some(id)));
        }
    }
    for (k, id) in &want {
        if !have.contains_key(k) && !k.starts_with(kopano_core::merge::CONFLICT_PREFIX) && info.owns(&Item::get(store, id)?.kind) {
            changes.push((k.clone(), None));
        }
    }
    if changes.is_empty() {
        return Ok((None, 0));
    }
    let n = changes.len();
    let kept_root = tree::update(store, root, changes)?;
    let kept = project::commit(store, pr, vec![h], kept_root, "Kept: this window's own state when it was linked again", cert, key)?;
    store.flush()?;
    for (k, id) in &want {
        let item = Item::get(store, id)?;
        if info.owns(&item.kind) && have.get(k) != Some(&item) {
            app.apply(k, Some(&item))?;
        }
    }
    // edit parts last: removing a clip's edit part removes the clip
    let mut extra: Vec<&String> = have.keys().filter(|k| !want.contains_key(*k)).collect();
    extra.sort_by_key(|k| k.ends_with("#edit"));
    for k in extra {
        app.apply(k, None)?;
    }
    Ok((Some(kept), n))
}

// ---- footage: serving ------------------------------------------------------------------------------

/// The Film engine that makes proxies: KOPANO_FILM_CLI, else next to kopanod, else on PATH.
pub fn film_cli() -> String {
    if let Ok(c) = std::env::var("KOPANO_FILM_CLI") {
        if !c.is_empty() {
            return c;
        }
    }
    let exe = if cfg!(windows) { "filmcraft-cli.exe" } else { "filmcraft-cli" };
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(exe))).filter(|p| p.is_file()).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| exe.into())
}

/// A proxy of `src` made by Film itself, in a headless engine (the owner's window is not
/// touched): H.264 at a quarter of the frame size, into `dir`. Returns the file.
pub fn make_proxy(cli: &str, src: &Path, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut t = Transport::headless(cli, None)?;
    t.exec("file.newProject", json!({"name": "proxy"}))?;
    let src_s = src.to_string_lossy().to_string();
    t.exec("file.import", json!({"paths": [src_s]}))?;
    let all = t.exec("kopano.items", json!({}))?;
    let item = all
        .get("media")
        .and_then(Value::as_array)
        .and_then(|ms| ms.iter().find(|m| m.get("path").and_then(Value::as_str).is_some_and(|p| Path::new(p) == src)))
        .and_then(|m| m.get("item").and_then(Value::as_u64))
        .context("Film did not import the original")?;
    let r = t.exec("media.createProxies", json!({"items": [item], "preset": PROXY_PRESET, "destination": dir.to_string_lossy(), "wait": true, "attach": false}))?;
    let out = r.pointer("/outputs/0/path").and_then(Value::as_str).context("Film made no proxy")?;
    if !Path::new(out).is_file() {
        bail!("Film's proxy job left no file at {out}");
    }
    Ok(PathBuf::from(out))
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Answer a member's `{"op":"media", "hash", "role": "info"|"preview"|"original"}` for Film.
pub async fn offer(node: &Arc<Node>, p: &Arc<Proj>, req: &Value) -> Result<Offer> {
    let m = node.media.get().context("media not started")?;
    let hash = req.get("hash").and_then(Value::as_str).context("hash")?.to_string();
    let originals = media::originals_in(&p.store, p.head())?;
    let Some(asset) = originals.values().find(|a| a.hash == hash).cloned() else { bail!("that is not footage of this project") };
    let path = with_slot(p, &node.device, |s, _, _| {
        s.conn.snapshot()?;
        Ok(s.conn.original_path(&hash))
    })
    .ok()
    .flatten();
    let known = state(p).media.get(&hash).cloned().unwrap_or_default();
    let name = path.as_deref().map(file_name).filter(|n| !n.is_empty()).unwrap_or(known.name.clone());
    match req.get("role").and_then(Value::as_str) {
        Some("info") => {
            if name.is_empty() {
                bail!("this computer does not know that footage's name");
            }
            Ok(Offer { hash, size: asset.size, name })
        }
        Some("original") => {
            if m.complete(&hash).await {
                return Ok(Offer { hash, size: asset.size, name });
            }
            let Some(path) = path else { bail!("the original is not on this computer") };
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
            if let Some(ph) = &known.proxy_hash {
                if m.complete(ph).await {
                    let size = m.store.remote().local(ph.parse::<iroh_blobs::Hash>()?).await?.local_bytes();
                    return Ok(Offer { hash: ph.clone(), size, name });
                }
            }
            let Some(path) = path else { bail!("no proxy of this footage on this computer") };
            let _one = m.render.lock().await;
            let made = m.made.lock().unwrap_or_else(|e| e.into_inner()).get(&hash).cloned();
            if let Some(o) = made {
                if m.complete(&o.hash).await {
                    return Ok(o);
                }
            }
            let dir = m.dir.join(format!("made-{}", &hash[..16]));
            let (cli, src, d2) = (film_cli(), PathBuf::from(&path), dir.clone());
            let t = Instant::now();
            let out = tokio::task::spawn_blocking(move || make_proxy(&cli, &src, &d2)).await??;
            let ms = t.elapsed().as_millis() as u64;
            let tag = m.store.blobs().add_path(&out).await?;
            let size = std::fs::metadata(&out)?.len();
            let _ = std::fs::remove_dir_all(&dir);
            let o = Offer { hash: tag.hash.to_hex().to_string(), size, name: name.clone() };
            m.remember_made(&hash, o.clone())?;
            m.refs.lock().unwrap_or_else(|e| e.into_inner()).remove(&p.project.id);
            node.emit(json!({"event": "preview-made", "project": p.project.id.hex(), "file": name, "kb": size / 1000, "ms": ms}));
            Ok(o)
        }
        _ => bail!("role must be info, preview or original"),
    }
}

// ---- footage: asking -------------------------------------------------------------------------------

/// The originals of the project's head, by hash.
fn footage(p: &Proj) -> Result<BTreeMap<String, kopano_core::project::AssetRef>> {
    Ok(media::originals_in(&p.store, p.head())?.into_values().map(|a| (a.hash.clone(), a)).collect())
}

/// How each original shows in the window now.
fn here(node: &Node, p: &Proj) -> Result<BTreeMap<String, MediaHere>> {
    with_slot(p, &node.device, |s, _, _| {
        s.conn.snapshot()?;
        Ok(s.conn.media_state.clone())
    })
}

/// Watch the project: footage the window lacks gets a proxy, waiting updates are tried again,
/// and on the device with the originals proxies are made ahead for teammates.
pub fn spawn(node: Arc<Node>, p: Arc<Proj>) {
    tokio::spawn(async move {
        let mut heads = p.heads.subscribe();
        let mut quiet = false;
        loop {
            let _ = tokio::time::timeout(Duration::from_secs(if quiet { 6 } else { 2 }), heads.recv()).await;
            if p.head().is_none() || node.media.get().is_none() {
                continue;
            }
            match round(&node, &p).await {
                Ok(busy) => quiet = !busy,
                Err(e) => {
                    node.emit(json!({"event": "media", "project": p.project.id.hex(), "error": format!("{e:#}")}));
                    tokio::time::sleep(Duration::from_secs(4)).await;
                }
            }
        }
    });
}

/// One round. Returns true while something is still missing or waiting.
async fn round(node: &Arc<Node>, p: &Arc<Proj>) -> Result<bool> {
    let pid = p.project.id.hex();
    let want = footage(p)?;
    let shown = here(node, p)?;
    let missing: Vec<(String, kopano_core::project::AssetRef)> = want.iter().filter(|(h, _)| !matches!(shown.get(*h), Some(MediaHere::Original { .. }))).map(|(h, a)| (h.clone(), a.clone())).collect();
    let mut st = state(p);
    // 1. names first: a media item whose file is missing here is then known as its original
    let mut learned = false;
    for (h, a) in &missing {
        if st.media.get(h).is_some_and(|m| !m.name.is_empty()) {
            continue;
        }
        for dev in media::sources(p) {
            if let Ok(v) = media::ask(node, p, &dev, &json!({"op": "media", "hash": h, "role": "info"})).await {
                if let Ok(o) = serde_json::from_value::<Offer>(v) {
                    let e = st.media.entry(h.clone()).or_default();
                    e.name = o.name;
                    e.size = a.size;
                    learned = true;
                    break;
                }
            }
        }
    }
    if learned {
        save_state(p, &st)?;
    }
    // 2. proxies for footage that has neither its original nor a proxy here
    let m = node.media.get().context("media not started")?.clone();
    let need: Vec<&(String, kopano_core::project::AssetRef)> = missing.iter().filter(|(h, _)| !st.media.get(h).and_then(|x| x.proxy.as_ref()).is_some_and(|f| Path::new(f).is_file())).collect();
    let (mut got, mut failed, mut bytes) = (0usize, 0usize, 0u64);
    let t0 = Instant::now();
    if !need.is_empty() {
        node.emit(json!({"event": "media", "project": pid, "files_waiting": need.len(), "files_ready": missing.len() - need.len()}));
    }
    for (h, a) in &need {
        let mut ok = None;
        for dev in media::sources(p) {
            let o = match media::ask(node, p, &dev, &json!({"op": "media", "hash": h, "role": "preview"})).await {
                Ok(v) => serde_json::from_value::<Offer>(v)?,
                Err(e) => {
                    eprintln!("[kopanod] media: {} has no proxy of {}: {e:#}", &dev[..8], &h[..8]);
                    continue;
                }
            };
            match media::fetch(node, &dev, &o.hash, |_, _| {}).await {
                Ok(n) => {
                    bytes += n;
                    ok = Some(o);
                    break;
                }
                Err(e) => eprintln!("[kopanod] media: proxy from {}: {e:#}", &dev[..8]),
            }
        }
        let Some(o) = ok else {
            failed += 1;
            continue;
        };
        let dir = m.preview_dir().join(&pid[..8]);
        std::fs::create_dir_all(&dir)?;
        let stem = Path::new(&o.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| h[..12].to_string());
        let path = media::file_name_for(&dir, &format!("{stem}_proxy.mp4"), None, &o.hash);
        m.store.blobs().export(o.hash.parse::<iroh_blobs::Hash>()?, &path).await?;
        let e = st.media.entry(h.clone()).or_default();
        if e.name.is_empty() {
            e.name = o.name.clone();
        }
        e.size = a.size;
        e.proxy = Some(path.to_string_lossy().to_string());
        e.proxy_hash = Some(o.hash.clone());
        save_state(p, &st)?;
        m.refs.lock().unwrap_or_else(|e| e.into_inner()).remove(&p.project.id);
        got += 1;
        // each clip plays as soon as its proxy is here
        let applied = take_in(node, p).await?;
        node.emit(json!({"event": "media", "project": pid, "files_waiting": need.len() - got - failed, "files_ready": missing.len() - need.len() + got,
            "file": o.name, "mb_done": (bytes as f64 / 1e4).round() / 100.0, "ms": t0.elapsed().as_millis() as u64, "applied": applied}));
    }
    // 3. attach proxies, try waiting updates again
    let applied = take_in(node, p).await?;
    if applied > 0 {
        node.emit(json!({"event": "media", "project": pid, "applied": applied}));
    }
    // 4. on the device with the originals: proxies ahead, so a teammate who joins waits less
    if missing.is_empty() {
        ahead(node, p, &want, &shown).await;
    }
    let waiting = with_slot(p, &node.device, |s, _, _| Ok(s.deferred.len()))?;
    Ok(!missing.is_empty() || waiting > 0 || failed > 0)
}

/// With nothing else writing: save the window's own edits, attach proxies that are here, then
/// give the window the updates it is still waiting for. Returns how many it took.
async fn take_in(node: &Arc<Node>, p: &Arc<Proj>) -> Result<usize> {
    let _w = p.write.lock().await;
    let st = state(p);
    let (saved, n) = with_slot(p, &node.device, |s, head, joining| {
        if joining {
            return Ok((None, 0));
        }
        let mut repo = p.repo(&node.device)?;
        let v = {
            let block = s.block;
            let mut view = View::new(&p.store, head, &mut s.conn, &mut s.deferred);
            view.expect_block = block;
            repo.save(&mut view, "Edit in Film")?
        };
        if v.is_some() {
            p.set_head(repo.head)?;
            crate::approve::sign_mine(node, p)?;
        }
        s.conn.snapshot()?;
        let mut attached = 0;
        for (h, mh) in s.conn.media_state.clone() {
            let MediaHere::Missing { item } = &mh else { continue };
            // an original received earlier (the window reopened its file): read it again
            if let Some(o) = st.media.get(&h).and_then(|x| x.original.clone()).filter(|f| Path::new(f).is_file()) {
                s.conn.t.exec("media.relink", json!({"item": item, "path": o, "force": true, "relinkOthers": false}))?;
                attached += 1;
                continue;
            }
            let Some(pr) = st.media.get(&h).and_then(|x| x.proxy.clone()).filter(|f| Path::new(f).is_file()) else { continue };
            if s.attached.contains(item) {
                continue;
            }
            s.conn.t.exec("media.attachProxies", json!({"item": item, "path": pr, "force": true}))?;
            s.attached.insert(*item);
            attached += 1;
        }
        if attached > 0 && !s.proxies_on {
            s.conn.t.exec("media.toggleProxies", json!({"enabled": true}))?;
            s.proxies_on = true;
        }
        let mut view = View::new(&p.store, p.head(), &mut s.conn, &mut s.deferred);
        let n = view.retry();
        if (n > 0 || attached > 0) && s.control {
            save_file(&mut s.conn);
        }
        Ok((v, n))
    })?;
    if saved.is_some() {
        if let Some(h) = p.head() {
            let _ = p.heads.send(h);
        }
        node.emit(json!({"event": "saved", "project": p.project.id.hex(), "head": p.head().map(|h| h.short()), "now_ms": kopano_core::identity::now_ms()}));
    }
    Ok(n)
}

async fn ahead(node: &Arc<Node>, p: &Arc<Proj>, want: &BTreeMap<String, kopano_core::project::AssetRef>, shown: &BTreeMap<String, MediaHere>) {
    let (Some(m), true) = (node.media.get(), p.project.genesis.members.len() > 1) else { return };
    if m.ahead_off.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    for h in want.keys() {
        if !matches!(shown.get(h), Some(MediaHere::Original { .. })) || m.made.lock().unwrap_or_else(|e| e.into_inner()).contains_key(h) {
            continue;
        }
        if let Err(e) = offer(node, p, &json!({"hash": h, "role": "preview"})).await {
            eprintln!("[kopanod] media: proxies are not made ahead on this device: {e:#}");
            m.ahead_off.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
    }
}

// ---- original on request ---------------------------------------------------------------------------

/// `fetch-original {project, key}`: key = an original's hash, a clip key, or "all".
pub fn fetch_original(node: &Arc<Node>, p: &Arc<Proj>, key: &str) -> Result<Value> {
    let want = footage(p)?;
    let hashes: Vec<String> = if key == "all" {
        want.keys().cloned().collect()
    } else if want.contains_key(key) {
        vec![key.to_string()]
    } else {
        let b = format!("{}#edit", base(key));
        let h = p.head().context("no project yet")?;
        let id = tree::get(&p.store, project::load_version(&p.store, &h)?.tree()?, &b)?.context("no such clip or footage in the project")?;
        vec![media_of(&Item::get(&p.store, &id)?).context("that clip has no footage")?.to_string()]
    };
    let shown = here(node, p)?;
    let m = node.media.get().context("media not started")?;
    let mut started = Vec::new();
    for h in hashes {
        if matches!(shown.get(&h), Some(MediaHere::Original { .. })) {
            continue;
        }
        {
            let mut jobs = m.jobs.lock().unwrap_or_else(|e| e.into_inner());
            if jobs.get(&h).is_some_and(|j| j["state"] == "downloading") {
                continue;
            }
            jobs.insert(h.clone(), json!({"state": "downloading", "mb_done": 0.0, "mb_total": want[&h].size as f64 / 1e6}));
        }
        started.push(h.clone());
        let (node, p, a) = (node.clone(), p.clone(), want[&h].clone());
        tokio::spawn(async move {
            let r = get_original(&node, &p, &a).await;
            let pid = p.project.id.hex();
            let ev = match &r {
                Ok(v) => json!({"event": "original", "project": pid, "key": a.hash, "state": "ready", "file": v["file"], "mb": v["mb"], "ms": v["ms"], "swap_ms": v["swap_ms"]}),
                Err(e) => json!({"event": "original", "project": pid, "key": a.hash, "state": "failed", "error": format!("{e:#}")}),
            };
            if let Some(m) = node.media.get() {
                m.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(a.hash.clone(), ev.clone());
            }
            node.emit(ev);
        });
    }
    Ok(json!({"started": started.len(), "keys": started}))
}

async fn get_original(node: &Arc<Node>, p: &Arc<Proj>, a: &kopano_core::project::AssetRef) -> Result<Value> {
    let m = node.media.get().context("media not started")?.clone();
    let pid = p.project.id.hex();
    let t = Instant::now();
    let mut last = Instant::now() - Duration::from_secs(5);
    let (mut errors, mut got) = (Vec::new(), None);
    for dev in media::sources(p) {
        let o = match media::ask(node, p, &dev, &json!({"op": "media", "hash": a.hash, "role": "original"})).await {
            Ok(v) => serde_json::from_value::<Offer>(v)?,
            Err(e) => {
                errors.push(format!("{}: {e:#}", &dev[..8]));
                continue;
            }
        };
        let r = media::fetch(node, &dev, &a.hash, |done, total| {
            if last.elapsed() >= Duration::from_secs(1) || done == total {
                last = Instant::now();
                let ev = json!({"event": "original", "project": pid, "key": a.hash, "file": o.name, "state": "downloading",
                    "mb_done": (done as f64 / 1e4).round() / 100.0, "mb_total": (total as f64 / 1e4).round() / 100.0});
                m.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(a.hash.clone(), ev.clone());
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
    let path = media::file_name_for(&dir, &o.name, None, &a.hash);
    let opts = iroh_blobs::api::blobs::ExportOptions { hash: a.hash.parse()?, mode: iroh_blobs::api::blobs::ExportMode::TryReference, target: std::path::absolute(&path)? };
    m.store.blobs().export_with_opts(opts).await?;
    let t2 = Instant::now();
    let mut st = state(p);
    let e = st.media.entry(a.hash.clone()).or_default();
    e.original = Some(path.to_string_lossy().to_string());
    if e.name.is_empty() {
        e.name = o.name.clone();
    }
    e.size = a.size;
    save_state(p, &st)?;
    {
        // the media item now reads the original (exports are made from it)
        let _w = p.write.lock().await;
        let ps = path.to_string_lossy().to_string();
        with_slot(p, &node.device, |s, _, _| {
            s.conn.snapshot()?;
            match s.conn.media_state.get(&a.hash).cloned() {
                Some(MediaHere::Missing { item }) | Some(MediaHere::StandIn { item, .. }) => {
                    s.conn.t.exec("media.relink", json!({"item": item, "path": ps, "force": true, "relinkOthers": false}))?;
                }
                _ => {}
            }
            if s.control {
                save_file(&mut s.conn);
            }
            Ok(())
        })?;
    }
    take_in(node, p).await?;
    Ok(json!({"file": o.name, "mb": (a.size as f64 / 1e4).round() / 100.0, "ms": ms, "swap_ms": t2.elapsed().as_millis() as u64}))
}

/// The `media` op for Film: where each original of the project stands on this device.
pub fn status(node: &Arc<Node>, p: &Arc<Proj>) -> Result<Value> {
    let want = footage(p)?;
    let shown = here(node, p)?;
    let st = state(p);
    let jobs: BTreeMap<String, Value> = node.media.get().map(|m| m.jobs.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default();
    let waiting = with_slot(p, &node.device, |s, _, _| Ok(s.deferred.keys().cloned().collect::<Vec<_>>()))?;
    let (mut orig, mut proxy, mut none) = (0, 0, 0);
    let files: Vec<Value> = want
        .iter()
        .map(|(h, a)| {
            let from = match shown.get(h) {
                Some(MediaHere::Original { .. }) => "original",
                Some(MediaHere::StandIn { .. }) => "proxy",
                Some(MediaHere::Missing { item }) if with_proxy(&st, h) && node_attached(p, *item) => "proxy",
                Some(MediaHere::Missing { .. }) => "missing",
                None => "waiting",
            };
            match from {
                "original" => orig += 1,
                "proxy" => proxy += 1,
                _ => none += 1,
            }
            json!({"key": h, "name": st.media.get(h).map(|m| m.name.clone()), "shown_from": from, "original_mb": (a.size as f64 / 1e4).round() / 100.0, "download": jobs.get(h)})
        })
        .collect();
    Ok(json!({"files": want.len(), "from_original": orig, "from_proxy": proxy, "waiting": none, "items_waiting": waiting, "block": with_slot(p, &node.device, |s, _, _| Ok(s.block))?, "items": files}))
}

fn with_proxy(st: &State, h: &str) -> bool {
    st.media.get(h).and_then(|m| m.proxy.as_ref()).is_some_and(|f| Path::new(f).is_file())
}
fn node_attached(p: &Proj, item: u64) -> bool {
    p.film.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.attached.contains(&item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kopano_core::connector::KindInfo;
    use kopano_core::identity::Provider;
    use kopano_core::project::AssetRef;
    use kopano_core::repo::Repo;

    fn clip(media: &str, cut: i64) -> Item {
        Item { kind: "film.clip".into(), lane: "edit".into(), schema: 1, state: json!({"track": 1, "cut": [0, cut, 0], "media": media}), assets: vec![AssetRef { hash: media.into(), size: 1000, role: "original".into() }] }
    }
    fn grade(exposure: f64) -> Item {
        Item { kind: "film.grade".into(), lane: "grade".into(), schema: 1, state: json!({"fx": {"lumetri": {"params": {"exposure": {"value": {"Float": exposure}}}}}}), assets: vec![] }
    }

    /// A pretend Film window: items by key; knows some media; can refuse keys.
    struct Fake {
        items: BTreeMap<String, Item>,
        media: BTreeSet<String>,
        refuse: BTreeSet<String>,
        blk: Option<u64>,
    }
    impl Connector for Fake {
        fn info(&self) -> ConnectorInfo {
            let k = |kind: &str, lane: &str| KindInfo { kind: kind.into(), lane: lane.into(), schema: 1, mergeable: true };
            ConnectorInfo { app: "ubunye.film".into(), contract: kopano_core::connector::CONTRACT_VERSION, kinds: vec![k("film.clip", "edit"), k("film.grade", "grade"), k("film.sound", "sound"), k("film.track", "edit"), k("film.mix", "sound")] }
        }
        fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
            Ok(self.items.clone())
        }
        fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
            if self.refuse.contains(key) {
                bail!("Film refused {key}");
            }
            if let Some(m) = item.and_then(media_of) {
                if !self.media.contains(m) {
                    bail!("media {m} is not on this computer yet");
                }
            }
            match item {
                Some(i) => self.items.insert(key.into(), i.clone()),
                None => self.items.remove(key),
            };
            Ok(())
        }
    }
    impl FilmApp for Fake {
        fn has_media(&self, hash: &str) -> bool {
            self.media.contains(hash)
        }
        fn block(&self) -> Option<u64> {
            self.blk
        }
    }

    struct Team {
        store: RedbStore,
        pr: Project,
        cert: DeviceCert,
        key: DeviceKey,
        dir: PathBuf,
    }
    impl Drop for Team {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    fn team(tag: &str) -> Result<Team> {
        let dir = std::env::temp_dir().join(format!("kopano-film-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let store = RedbStore::open(dir.join("k.redb"))?;
        let provider = Provider::generate();
        let key = DeviceKey::generate();
        let cert = provider.issue("acct-a", "a", &key, "pc", 3_600_000)?;
        let mut members = BTreeMap::new();
        members.insert("acct-a".to_string(), project::Member { role: project::Role::Editor, lanes: vec![] });
        let pr = Project::create(&store, "film", vec![provider.public_hex()], &cert, &key, members)?;
        Ok(Team { store, pr, cert, key, dir })
    }
    fn commit(t: &Team, items: &[(&str, Item)]) -> Result<Id> {
        let mut changes = Vec::new();
        for (k, it) in items {
            changes.push((k.to_string(), Some(it.put(&t.store)?)));
        }
        let root = tree::update(&t.store, None, changes)?;
        let v = project::commit(&t.store, &t.pr, vec![], root, "start", &t.cert, &t.key)?;
        t.store.flush()?;
        Ok(v)
    }
    fn repo(t: &Team, head: Option<Id>) -> Repo<'_> {
        Repo { store: &t.store, project: Project::load(&t.store, &t.pr.id).unwrap(), head, cert: t.cert.clone(), key: DeviceKey::from_bytes(t.key.to_bytes()) }
    }

    #[test]
    fn id_blocks_are_in_range_and_differ_between_devices_and_tries() {
        let pid = Id::parse(&"ab".repeat(32)).unwrap();
        let mut seen = BTreeSet::new();
        for d in ["c013b977", "6c1be93e", "11223344"] {
            for n in 0..50 {
                let b = derive_block(d, &pid, n);
                assert!((1..=MAX_ID_BLOCK).contains(&b));
                assert!(seen.insert(b), "block {b} came twice");
            }
        }
        assert_eq!(derive_block("c013b977", &pid, 3), derive_block("c013b977", &pid, 3), "the same device asks for the same block again");
    }

    /// The data loss this module must not cause: a joining window's own copy of the project
    /// file saved as a second history before the team's version has arrived.
    #[test]
    fn a_joining_window_says_nothing_until_the_project_is_here() -> Result<()> {
        let t = team("join")?;
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 100))]), media: BTreeSet::from(["b3-a".into()]), refuse: BTreeSet::new(), blk: None };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, None, &mut app, &mut deferred);
        v.joining = true;
        assert_eq!(repo(&t, None).save(&mut v, "Edit in Film")?, None);
        Ok(())
    }

    #[test]
    fn a_clip_whose_footage_is_not_here_is_the_projects_clip() -> Result<()> {
        let t = team("waiting")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 100)), ("film/1/clip/7#grade", grade(0.9)), ("film/1/clip/8#edit", clip("b3-b", 50))])?;
        // the window has clip 8 (footage here) and an unresolved copy of clip 7
        let mut raw = clip("b3-a", 100);
        raw.state = json!({"track": 1, "cut": [0, 100, 0], "item": 5923690164256769u64});
        raw.assets.clear();
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), raw), ("film/1/clip/8#edit".into(), clip("b3-b", 50))]), media: BTreeSet::from(["b3-b".into()]), refuse: BTreeSet::new(), blk: None };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        assert_eq!(repo(&t, Some(h)).save(&mut v, "Edit in Film")?, None, "nothing changed: clip 7 waits for its footage");
        // an edit to clip 8 is saved, clip 7 (and its grade) stay as the project has them
        v.app.items.get_mut("film/1/clip/8#edit").unwrap().state["cut"] = json!([0, 40, 10]);
        let nv = repo(&t, Some(h)).save(&mut v, "Edit in Film")?.expect("saved");
        let now = tree::entries(&t.store, project::load_version(&t.store, &nv)?.tree()?)?;
        assert_eq!(now.len(), 3);
        assert_eq!(Item::get(&t.store, &now["film/1/clip/7#grade"])?, grade(0.9));
        assert_eq!(Item::get(&t.store, &now["film/1/clip/7#edit"])?, clip("b3-a", 100));
        Ok(())
    }

    #[test]
    fn a_refused_update_waits_and_is_tried_again() -> Result<()> {
        let t = team("retry")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 100))])?;
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 100))]), media: BTreeSet::from(["b3-a".into()]), refuse: BTreeSet::from(["film/1/clip/7#grade".into()]), blk: None };
        let mut deferred = BTreeMap::new();
        {
            let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
            v.apply("film/1/clip/7#grade", Some(&grade(0.5)))?;
            assert_eq!(v.applied, 0);
            // the window's refusal is not read as "the user removed the grade"
            assert_eq!(v.snapshot()?.get("film/1/clip/7#grade"), Some(&grade(0.5)));
        }
        assert_eq!(deferred.len(), 1);
        app.refuse.clear();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        assert_eq!(v.retry(), 1);
        assert!(v.deferred.is_empty());
        assert_eq!(v.app.items.get("film/1/clip/7#grade"), Some(&grade(0.5)));
        Ok(())
    }

    /// The user grades a clip whose team grade the window refused: the user's grade is the later
    /// edit and is saved. Before, the waiting update hid it from every save and the next retry
    /// put the team's grade over it, so it was never saved anywhere.
    #[test]
    fn a_user_edit_on_a_waiting_update_wins() -> Result<()> {
        let t = team("waiting-user")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 100)), ("film/1/clip/7#grade", grade(0.1))])?;
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 100)), ("film/1/clip/7#grade".into(), grade(0.1))]), media: BTreeSet::from(["b3-a".into()]), refuse: BTreeSet::from(["film/1/clip/7#grade".into()]), blk: None };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        v.snapshot()?;
        v.apply("film/1/clip/7#grade", Some(&grade(0.5)))?;
        assert_eq!(v.deferred["film/1/clip/7#grade"].window, Some(grade(0.1)));
        // the user moves the grade in the window
        v.app.items.insert("film/1/clip/7#grade".into(), grade(0.9));
        let nv = repo(&t, Some(h)).save(&mut v, "Edit in Film")?.expect("the user's grade is saved");
        assert_eq!(Item::get(&t.store, &tree::get(&t.store, project::load_version(&t.store, &nv)?.tree()?, "film/1/clip/7#grade")?.unwrap())?, grade(0.9));
        assert!(v.deferred.is_empty());
        v.app.refuse.clear();
        assert_eq!(v.retry(), 0);
        assert_eq!(v.app.items["film/1/clip/7#grade"], grade(0.9));
        Ok(())
    }

    #[test]
    fn a_relinked_window_is_brought_to_the_project_and_its_own_state_kept() -> Result<()> {
        let t = team("relink")?;
        // the team trimmed clip 7, graded it, and removed clip 9
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 80)), ("film/1/clip/7#grade", grade(0.9))])?;
        // a fresh window opened the old file: clip 7 untrimmed, no grade, clip 9 still there
        let mut app = Fake {
            items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 100)), ("film/1/clip/9#edit".into(), clip("b3-a", 30)), ("film/1/clip/9#grade".into(), grade(0.1))]),
            media: BTreeSet::from(["b3-a".into()]),
            refuse: BTreeSet::new(),
            blk: None,
        };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        let (kept, n) = bring_to_head(&t.store, &t.pr, &t.cert, &t.key, h, &mut v)?;
        assert_eq!(n, 4);
        assert_eq!(v.snapshot()?, tree::entries(&t.store, project::load_version(&t.store, &h)?.tree()?)?.into_iter().map(|(k, id)| (k, Item::get(&t.store, &id).unwrap())).collect());
        // nothing is saved afterwards: the window equals the project
        assert_eq!(repo(&t, Some(h)).save(&mut v, "Edit in Film")?, None);
        // the window's own state is a version off the main line, with clip 9 in it
        let k = kept.expect("kept");
        let kept_items = tree::entries(&t.store, project::load_version(&t.store, &k)?.tree()?)?;
        assert!(kept_items.contains_key("film/1/clip/9#edit"));
        assert!(!kept_items.contains_key("film/1/clip/7#grade"));
        assert_eq!(project::load_version(&t.store, &k)?.parents()?, vec![h]);
        Ok(())
    }

    #[test]
    fn a_window_equal_to_the_project_keeps_nothing() -> Result<()> {
        let t = team("same")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 80))])?;
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 80))]), media: BTreeSet::from(["b3-a".into()]), refuse: BTreeSet::new(), blk: None };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        assert_eq!(bring_to_head(&t.store, &t.pr, &t.cert, &t.key, h, &mut v)?, (None, 0));
        Ok(())
    }

    /// S6 found this race: a fresh window opened an old file after it was brought to the
    /// project, and its old state was saved and pushed. A snapshot from another id block (another
    /// file) is refused, so nothing it shows is saved before it is linked again.
    #[test]
    fn a_window_that_opened_another_file_is_never_saved() -> Result<()> {
        let t = team("block")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 80))])?;
        let mut app = Fake { items: BTreeMap::from([("film/1/clip/7#edit".into(), clip("b3-a", 100))]), media: BTreeSet::from(["b3-a".into()]), refuse: BTreeSet::new(), blk: Some(77) };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        v.expect_block = Some(42);
        assert!(repo(&t, Some(h)).save(&mut v, "Edit in Film").is_err());
        v.expect_block = Some(77);
        assert!(repo(&t, Some(h)).save(&mut v, "Edit in Film")?.is_some(), "the window's own edit in its own block is saved");
        Ok(())
    }

    #[test]
    fn a_window_with_no_project_open_removes_nothing() -> Result<()> {
        let t = team("empty")?;
        let h = commit(&t, &[("film/1/clip/7#edit", clip("b3-a", 80)), ("film/1/clip/7#grade", grade(0.9))])?;
        let mut app = Fake { items: BTreeMap::new(), media: BTreeSet::new(), refuse: BTreeSet::new(), blk: None };
        let mut deferred = BTreeMap::new();
        let mut v = View::new(&t.store, Some(h), &mut app, &mut deferred);
        v.joining = true;
        assert_eq!(repo(&t, Some(h)).save(&mut v, "Edit in Film")?, None, "an empty window is not \"every clip removed\"");
        Ok(())
    }
}
