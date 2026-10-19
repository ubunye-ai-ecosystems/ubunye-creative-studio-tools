//! Kopano connector for Ubunye Photo (the retouching app): teammates co-edit one Photo document.
//!
//! It drives Photo through Photo's own automation surfaces, like the Light connector:
//! - `Transport::Lines`: `photocraft-cli serve` headless engine over stdio (tests, no window), or
//! - `Transport::Tcp`: the running desktop app's control port (`photocraft --control PORT`,
//!   first line authenticates with the control token).
//!
//! Photo saves a document as a `.pcraft` bundle: a manifest plus content-addressed 256x256 tiles,
//! each already named by the BLAKE3 hash of its pixels. Photo's `kopano.*` commands (Ubunye
//! Photo, `crates/engine/src/ubunye_kopano.rs`) hand that bundle over the control channel without
//! touching the disk. The connector turns it into items:
//!
//! | key | kind | lane | merge |
//! |---|---|---|---|
//! | `doc` | `photo.document` | layers | setting by setting; the layer tree is one value |
//! | `layer/<uid>` | `photo.raster` / `photo.adjustment` / `photo.text` / `photo.layer` | retouch / adjust / text / layers | setting by setting |
//! | `<owner>@<surface>@<tx>_<ty>` | `photo.tile.<lane>` | the owner's lane | never: same tile = a clash |
//!
//! A tile item holds only its pixel hash; the pixels are an asset (`role: "tile"`, the raw
//! little-endian samples, so the asset hash IS Photo's tile hash). A brush stroke changes only the
//! tile items it touched, and only those tiles move. Two people painting different tiles of the
//! same layer merge cleanly.
//!
//! Stable keys: a layer's `uid` is never Photo's local layer id. Layers a document already has the
//! first time the connector sees it get a uid derived from their place and name (so two people who
//! open the same document agree); layers made later get a random uid. The map from local id to
//! uid is kept by the connector and written into the document (`kopano.layers` in its text
//! metadata) on every update, so it travels with the `.pcraft` file.
//!
//! Media: every tile and blob the items name is kept in `media/<blake3>` (raw bytes). Before
//! `apply` the node must have fetched the assets an item names into that folder.
//!
//! Changes are applied lazily: `apply` updates the connector's model, and the next `snapshot`
//! (or `flush`) sends one update to Photo, which is one undo step there.

use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::Engine as _;
use kopano_core::conformance::Harness;
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::project::{AssetRef, Item};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const UID_META: &str = "kopano.layers";
/// Raw bytes of zstd objects per `kopano.stage` request (control requests are at most 1 MiB).
const STAGE_BATCH: usize = 600 << 10;

pub enum Transport {
    Lines { child: Child, stdin: ChildStdin, stdout: BufReader<ChildStdout>, next: u64 },
    Tcp { reader: BufReader<std::net::TcpStream>, writer: std::net::TcpStream, next: u64 },
}

impl Transport {
    /// A headless Photo engine (`photocraft-cli serve`), in memory only, no files.
    pub fn headless(cli: &str) -> Result<Self> {
        let mut child = Command::new(cli)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("cannot start {cli} serve"))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("no stdout")?);
        Ok(Transport::Lines { child, stdin, stdout, next: 0 })
    }

    /// The running desktop app's control port; `token` is the content of its control token file.
    pub fn control(port: u16, token: &str) -> Result<Self> {
        let s = std::net::TcpStream::connect(("127.0.0.1", port)).with_context(|| format!("Ubunye Photo is not listening on control port {port}"))?;
        let mut t = Transport::Tcp { reader: BufReader::new(s.try_clone()?), writer: s, next: 0 };
        t.call("auth", json!({"token": token.trim()})).context("Photo refused the control token")?;
        Ok(t)
    }

    /// One request, one reply (same envelope on stdio and TCP).
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let (w, r, next): (&mut dyn Write, &mut dyn BufRead, &mut u64) = match self {
            Transport::Lines { stdin, stdout, next, .. } => (stdin, stdout, next),
            Transport::Tcp { reader, writer, next } => (writer, reader, next),
        };
        *next += 1;
        let id = *next;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        ensure!(line.len() < 1 << 20, "request for {method} is over Photo's 1 MiB limit");
        writeln!(w, "{line}")?;
        w.flush()?;
        loop {
            let mut buf = String::new();
            if r.read_line(&mut buf)? == 0 {
                bail!("Photo stopped answering");
            }
            let v: Value = match serde_json::from_str(&buf) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id") != Some(&json!(id)) {
                continue;
            }
            if v.get("ok") != Some(&Value::Bool(true)) {
                bail!("Photo could not run {method}: {}", v.get("error").and_then(Value::as_str).unwrap_or("?"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Run one engine command by id.
    pub fn exec(&mut self, command: &str, params: Value) -> Result<Value> {
        self.call("engine.execute", json!({"command": command, "params": params})).with_context(|| format!("Photo command {command}"))
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if let Transport::Lines { child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Bytes the connector moved to or from Photo (for the measurements).
#[derive(Default, Clone, Debug)]
pub struct Traffic {
    pub fetched_objects: u64,
    pub fetched_zstd_bytes: u64,
    pub staged_objects: u64,
    pub staged_zstd_bytes: u64,
    pub updates: u64,
}

pub struct PhotoConnector {
    pub t: Transport,
    cli: String,
    media: PathBuf,
    /// The bundle manifest of Photo's document at the last snapshot.
    manifest: Value,
    /// The items Photo holds (last snapshot), with `apply` changes on top while `dirty`.
    items: BTreeMap<String, Item>,
    loaded: bool,
    dirty: bool,
    /// Photo local layer id -> uid.
    uids: BTreeMap<u64, String>,
    seeded: bool,
    mint: u64,
    /// Photo's document revision at the last load; an update is refused if it moved since.
    revision: u64,
    pub traffic: Traffic,
    sample_key: String,
}

fn hex16(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(&[0]);
    }
    h.finalize().to_hex()[..16].to_string()
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lane and kind of a layer from its content kind.
fn layer_kind(layer: &Value) -> (&'static str, &'static str) {
    match layer.pointer("/content/kind").and_then(Value::as_str) {
        Some("raster") => ("photo.raster", "retouch"),
        Some("adjustment") => ("photo.adjustment", "adjust"),
        Some("text") => ("photo.text", "text"),
        _ => ("photo.layer", "layers"),
    }
}

fn pointer_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Take every surface's tiles out of `v` (a surface is `{format, default, tiles: [...]}`),
/// returning (JSON pointer of the surface, tx, ty, hash). Surfaces keep `tiles: []`.
fn strip_tiles(v: &mut Value, at: &str, out: &mut Vec<(String, i64, i64, String)>) {
    match v {
        Value::Object(m) => {
            if m.contains_key("format") && m.contains_key("default") && m.get("tiles").is_some_and(Value::is_array) {
                if let Some(Value::Array(tiles)) = m.get_mut("tiles") {
                    for t in tiles.drain(..) {
                        if let (Some(tx), Some(ty), Some(h)) = (t["tx"].as_i64(), t["ty"].as_i64(), t["hash"].as_str()) {
                            out.push((at.to_string(), tx, ty, h.to_string()));
                        }
                    }
                }
            }
            for (k, x) in m.iter_mut() {
                strip_tiles(x, &format!("{at}/{}", pointer_escape(k)), out);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter_mut().enumerate() {
                strip_tiles(x, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}

/// Every BLAKE3 hex string in `v` (blob references: ICC, EXIF, PSD blocks, smart objects).
fn blob_refs(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) if is_hash(s) => {
            out.insert(s.clone());
        }
        Value::Object(m) => m.values().for_each(|x| blob_refs(x, out)),
        Value::Array(a) => a.iter().for_each(|x| blob_refs(x, out)),
        _ => {}
    }
}

fn tile_key(owner: &str, surface: &str, tx: i64, ty: i64) -> String {
    format!("{owner}@{surface}@{tx}_{ty}")
}

fn parse_tile_key(key: &str) -> Option<(&str, &str, i64, i64)> {
    let mut it = key.splitn(3, '@');
    let (owner, surface, pos) = (it.next()?, it.next()?, it.next()?);
    let (tx, ty) = pos.split_once('_')?;
    Some((owner, surface, tx.parse().ok()?, ty.parse().ok()?))
}

impl PhotoConnector {
    pub fn new(t: Transport, cli: &str, media: &Path) -> Result<Self> {
        std::fs::create_dir_all(media)?;
        Ok(Self {
            t,
            cli: cli.into(),
            media: media.into(),
            manifest: Value::Null,
            items: BTreeMap::new(),
            loaded: false,
            dirty: false,
            uids: BTreeMap::new(),
            seeded: false,
            mint: 0,
            revision: 0,
            traffic: Traffic::default(),
            sample_key: String::new(),
        })
    }

    /// The running desktop app, attached to its active document.
    pub fn live(port: u16, token: &str, media: &Path) -> Result<Self> {
        Self::new(Transport::control(port, token)?, "", media)
    }

    /// A headless engine with the conformance demo document: a white background, a painted layer
    /// and an Exposure adjustment layer. Every fresh one is identical.
    pub fn headless_demo(cli: &str, media: &Path) -> Result<Self> {
        let mut c = Self::new(Transport::headless(cli)?, cli, media)?;
        c.t.exec("file.new", json!({"width": 640, "height": 400, "background": "white", "name": "Kopano demo"}))?;
        c.t.exec("layer.new.layer", json!({"name": "Retouch"}))?;
        c.t.exec("paint.stroke", json!({"points": [[40, 40, 1], [300, 120, 1], [600, 360, 1]], "size": 24, "color": "#c03020"}))?;
        c.t.exec("layer.newAdjustmentLayer.exposure", json!({"exposure": 0.0, "offset": 0.0, "gamma": 1.0}))?;
        let snap = c.snapshot()?;
        c.sample_key = snap.iter().find(|(_, i)| i.kind == "photo.adjustment").map(|(k, _)| k.clone()).context("demo has no adjustment layer")?;
        Ok(c)
    }

    pub fn media_path(&self, hash: &str) -> PathBuf {
        self.media.join(hash)
    }

    fn fresh_uid(&mut self, name: &str) -> String {
        self.mint += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        hex16(&["photo-layer-new", name, &now.to_string(), &std::process::id().to_string(), &self.mint.to_string(), &rand::random::<u64>().to_string()])
    }

    /// Ask Photo for its document and rebuild the items from it.
    fn load(&mut self) -> Result<()> {
        let r = self.t.exec("kopano.export", json!({}))?;
        let manifest = r.get("manifest").cloned().context("Photo sent no manifest")?;
        let doc = manifest.get("document").context("manifest has no document")?;
        // The uid map stored in the document (written by an earlier update) fills the gaps.
        let stored: Option<BTreeMap<u64, String>> = doc
            .pointer("/metadata/text")
            .and_then(Value::as_array)
            .and_then(|t| t.iter().find(|p| p[0] == UID_META))
            .and_then(|p| p[1].as_str())
            .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(s).ok())
            .map(|m| m.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect());
        let seeding = !self.seeded && stored.is_none();
        for (k, v) in stored.unwrap_or_default() {
            self.uids.entry(k).or_insert(v);
        }
        let mut items = BTreeMap::new();
        let mut used = BTreeSet::new();
        let mut alive = BTreeSet::new();
        let tree = self.layers_to_items(doc.get("layers").and_then(Value::as_array).cloned().unwrap_or_default(), "", seeding, &mut used, &mut alive, &mut items)?;
        self.uids.retain(|id, _| alive.contains(id));
        self.seeded = true;
        // The document item: everything but local state (selection, Quick Mask, its id and tab
        // name) and the layers, which are their own items.
        let mut d = doc.clone();
        if let Some(m) = d.as_object_mut() {
            for k in ["id", "name", "layers", "selection", "quick_mask"] {
                m.remove(k);
            }
            m.insert("tree".into(), Value::Array(tree));
        }
        if let Some(meta) = d.get_mut("metadata").and_then(Value::as_object_mut) {
            if let Some(Value::Array(t)) = meta.get_mut("text") {
                t.retain(|p| p[0] != UID_META);
                // Photo leaves out an empty list; so does the item.
                if t.is_empty() {
                    meta.remove("text");
                }
            }
        }
        self.add_owner_items("doc", "photo.document", "layers", d, &mut items)?;
        self.ensure_media(&items)?;
        self.size_assets(&mut items)?;
        self.revision = r.get("revision").and_then(Value::as_u64).unwrap_or(0);
        self.manifest = manifest;
        self.items = items;
        self.loaded = true;
        self.dirty = false;
        Ok(())
    }

    /// Layer items for one level of the tree (bottom to top); returns that level's tree nodes.
    fn layers_to_items(
        &mut self,
        layers: Vec<Value>,
        parent: &str,
        seeding: bool,
        used: &mut BTreeSet<String>,
        alive: &mut BTreeSet<u64>,
        items: &mut BTreeMap<String, Item>,
    ) -> Result<Vec<Value>> {
        let mut nodes = Vec::new();
        let mut same_name: BTreeMap<String, usize> = BTreeMap::new();
        for mut l in layers {
            let id = l.get("id").and_then(Value::as_u64).context("layer without id")?;
            let name = l.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let ordinal = {
                let n = same_name.entry(name.clone()).or_default();
                *n += 1;
                *n
            };
            let mut uid = match self.uids.get(&id) {
                Some(u) => u.clone(),
                None if seeding => hex16(&["photo-layer", parent, &name, &ordinal.to_string()]),
                None => self.fresh_uid(&name),
            };
            if used.contains(&uid) {
                // A duplicated layer carries no uid of its own yet.
                uid = self.fresh_uid(&name);
            }
            used.insert(uid.clone());
            alive.insert(id);
            self.uids.insert(id, uid.clone());
            let children = match l.pointer_mut("/content/children") {
                Some(Value::Array(c)) => Some(std::mem::take(c)),
                _ => None,
            };
            let mut node = json!({"uid": uid});
            if let Some(children) = children {
                node["children"] = Value::Array(self.layers_to_items(children, &uid, seeding, used, alive, items)?);
                if let Some(m) = l.get_mut("content").and_then(Value::as_object_mut) {
                    m.remove("children");
                }
            }
            if let Some(m) = l.as_object_mut() {
                m.remove("id");
            }
            let (kind, lane) = layer_kind(&l);
            self.add_owner_items(&format!("layer/{uid}"), kind, lane, l, items)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// The owner item (tiles stripped, blobs as assets) and one item per tile.
    fn add_owner_items(&self, owner: &str, kind: &str, lane: &str, mut state: Value, items: &mut BTreeMap<String, Item>) -> Result<()> {
        let mut tiles = Vec::new();
        strip_tiles(&mut state, "", &mut tiles);
        let mut blobs = BTreeSet::new();
        blob_refs(&state, &mut blobs);
        let assets = blobs.into_iter().map(|h| AssetRef { size: 0, hash: h, role: "blob".into() }).collect();
        items.insert(owner.to_string(), Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets });
        for (surface, tx, ty, hash) in tiles {
            items.insert(
                tile_key(owner, &surface, tx, ty),
                Item { kind: format!("photo.tile.{lane}"), lane: lane.into(), schema: 1, state: json!({"hash": hash}), assets: vec![AssetRef { size: 0, hash, role: "tile".into() }] },
            );
        }
        Ok(())
    }

    /// Fetch every tile and blob the items name that `media/` lacks, and fill in asset sizes.
    fn ensure_media(&mut self, items: &BTreeMap<String, Item>) -> Result<()> {
        let mut want: Vec<String> = items.values().flat_map(|i| i.assets.iter().map(|a| a.hash.clone())).collect::<BTreeSet<_>>().into_iter().filter(|h| !self.media_path(h).exists()).collect();
        while !want.is_empty() {
            let r = self.t.exec("kopano.objects", json!({"hashes": want}))?;
            let objs = r.get("objects").and_then(Value::as_object).cloned().unwrap_or_default();
            ensure!(!objs.is_empty() || r["missing"].as_array().is_some_and(|m| !m.is_empty()), "Photo returned no objects");
            for (h, b64) in &objs {
                let zst = base64::engine::general_purpose::STANDARD.decode(b64.as_str().unwrap_or(""))?;
                let mut raw = Vec::new();
                ruzstd::decoding::StreamingDecoder::new(zst.as_slice()).map_err(|e| anyhow!("tile {h}: {e}"))?.take(1 << 30).read_to_end(&mut raw)?;
                ensure!(blake3::hash(&raw).to_hex().as_str() == h, "Photo sent object {h} whose bytes do not match its hash");
                self.traffic.fetched_objects += 1;
                self.traffic.fetched_zstd_bytes += zst.len() as u64;
                write_media(&self.media_path(h), &raw)?;
            }
            if let Some(m) = r["missing"].as_array().filter(|m| !m.is_empty()) {
                bail!("Photo has no object {} its document names", m[0]);
            }
            want = r["more"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        }
        Ok(())
    }

    fn size_assets(&self, items: &mut BTreeMap<String, Item>) -> Result<()> {
        for i in items.values_mut() {
            for a in &mut i.assets {
                a.size = std::fs::metadata(self.media_path(&a.hash)).with_context(|| format!("media {} is not here", a.hash))?.len();
            }
        }
        Ok(())
    }

    /// Send pending changes to Photo as one update (one undo step there).
    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let manifest = self.build_manifest()?;
        // Objects Photo does not hold yet travel first, zstd-compressed, a few per request.
        let (mut have, mut _b) = (BTreeSet::new(), BTreeSet::new());
        collect_hashes(&self.manifest, &mut have, &mut _b);
        have.extend(_b);
        let (mut need, mut nb) = (BTreeSet::new(), BTreeSet::new());
        collect_hashes(&manifest, &mut need, &mut nb);
        need.extend(nb);
        let mut batch = Map::new();
        let mut batch_bytes = 0usize;
        for h in need.difference(&have) {
            let raw = std::fs::read(self.media_path(h)).with_context(|| format!("media {h} is not here yet; fetch it before applying"))?;
            ensure!(blake3::hash(&raw).to_hex().as_str() == h, "media file {h} is damaged");
            let zst = ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
            if batch_bytes + zst.len() > STAGE_BATCH && !batch.is_empty() {
                self.t.exec("kopano.stage", json!({"objects": std::mem::take(&mut batch)}))?;
                batch_bytes = 0;
            }
            self.traffic.staged_objects += 1;
            self.traffic.staged_zstd_bytes += zst.len() as u64;
            batch_bytes += zst.len();
            batch.insert(h.clone(), json!(base64::engine::general_purpose::STANDARD.encode(&zst)));
        }
        if !batch.is_empty() {
            self.t.exec("kopano.stage", json!({"objects": batch}))?;
        }
        // Refused when someone edited the document since the snapshot this update builds on; the
        // caller saves that edit and brings the changes in again.
        self.t.exec("kopano.import", json!({"manifest": manifest, "revision": self.revision})).context("Photo refused the update")?;
        self.traffic.updates += 1;
        self.dirty = false;
        self.loaded = false;
        Ok(())
    }

    /// The bundle manifest for the current items, keeping Photo's local state (ids, selection).
    fn build_manifest(&mut self) -> Result<Value> {
        let local = self.manifest.get("document").cloned().context("no document loaded")?;
        let doc_item = self.items.get("doc").cloned().context("the project has no Photo document item")?;
        // Tiles by owner and surface.
        let mut tiles: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
        for (k, i) in &self.items {
            if let Some((owner, surface, tx, ty)) = parse_tile_key(k) {
                tiles.entry((owner.to_string(), surface.to_string())).or_default().push(json!({"tx": tx, "ty": ty, "hash": i.state["hash"]}));
            }
        }
        let by_uid: BTreeMap<String, u64> = self.uids.iter().map(|(id, u)| (u.clone(), *id)).collect();
        let mut next_id = self.uids.keys().max().copied().unwrap_or(0).max(max_layer_id(&local)) + 1;
        let mut placed = BTreeSet::new();
        let tree = doc_item.state.get("tree").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut layers = Vec::new();
        let mut new_ids = Vec::new();
        for node in &tree {
            if let Some(l) = self.build_layer(node, &tiles, &by_uid, &mut next_id, &mut placed, &mut new_ids, 0)? {
                layers.push(l);
            }
        }
        // Layer items the tree does not name (two people added layers at once and the tree
        // clashed): keep them, on top.
        let orphans: Vec<String> = self.items.keys().filter_map(|k| k.strip_prefix("layer/")).filter(|u| !u.contains('@') && !placed.contains(*u)).map(String::from).collect();
        for uid in orphans {
            if let Some(l) = self.build_layer(&json!({"uid": uid}), &tiles, &by_uid, &mut next_id, &mut placed, &mut new_ids, 0)? {
                layers.push(l);
            }
        }
        for (id, uid) in new_ids {
            self.uids.insert(id, uid);
        }
        let mut d = doc_item.state.clone();
        restore_tiles(&mut d, "doc", &tiles);
        let m = d.as_object_mut().context("document item is not an object")?;
        m.remove("tree");
        for k in ["id", "name", "selection", "quick_mask"] {
            match local.get(k) {
                Some(v) => m.insert(k.into(), v.clone()),
                None => m.remove(k),
            };
        }
        m.insert("layers".into(), Value::Array(layers));
        let uid_map: BTreeMap<String, String> = self.uids.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let text = d.pointer_mut("/metadata/text").and_then(Value::as_array_mut);
        let entry = json!([UID_META, serde_json::to_string(&uid_map)?]);
        match text {
            Some(t) => t.push(entry),
            None => {
                if let Some(meta) = d.get_mut("metadata").and_then(Value::as_object_mut) {
                    meta.insert("text".into(), json!([entry]));
                }
            }
        }
        let mut manifest = self.manifest.clone();
        manifest["document"] = d;
        manifest["thumbnail"] = Value::Null;
        manifest["composite"] = Value::Null;
        Ok(manifest)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_layer(
        &self,
        node: &Value,
        tiles: &BTreeMap<(String, String), Vec<Value>>,
        by_uid: &BTreeMap<String, u64>,
        next_id: &mut u64,
        placed: &mut BTreeSet<String>,
        new_ids: &mut Vec<(u64, String)>,
        depth: usize,
    ) -> Result<Option<Value>> {
        ensure!(depth < 200, "layer tree is too deep");
        let Some(uid) = node.get("uid").and_then(Value::as_str) else { return Ok(None) };
        let owner = format!("layer/{uid}");
        let Some(item) = self.items.get(&owner) else { return Ok(None) };
        if !placed.insert(uid.to_string()) {
            return Ok(None);
        }
        let mut l = item.state.clone();
        restore_tiles(&mut l, &owner, tiles);
        let id = match by_uid.get(uid) {
            Some(id) => *id,
            None => {
                let id = *next_id;
                *next_id += 1;
                new_ids.push((id, uid.to_string()));
                id
            }
        };
        l["id"] = json!(id);
        if l.pointer("/content/kind").and_then(Value::as_str) == Some("group") {
            let mut kids = Vec::new();
            for c in node.get("children").and_then(Value::as_array).into_iter().flatten() {
                if let Some(k) = self.build_layer(c, tiles, by_uid, next_id, placed, new_ids, depth + 1)? {
                    kids.push(k);
                }
            }
            l["content"]["children"] = Value::Array(kids);
        }
        Ok(Some(l))
    }

    fn ensure_loaded(&mut self) -> Result<()> {
        if !self.loaded {
            self.load()?;
        }
        Ok(())
    }

    /// Photo's local id for a layer uid (after pending changes were sent).
    pub fn layer_id(&mut self, uid: &str) -> Result<u64> {
        self.flush()?;
        self.ensure_loaded()?;
        self.uids.iter().find(|(_, u)| u.as_str() == uid).map(|(id, _)| *id).with_context(|| format!("Photo has no layer {uid}"))
    }

    /// The local manifest of the layer with this local id.
    fn local_layer(&self, id: u64) -> Option<Value> {
        fn find(layers: &[Value], id: u64) -> Option<Value> {
            for l in layers {
                if l["id"].as_u64() == Some(id) {
                    return Some(l.clone());
                }
                if let Some(c) = l.pointer("/content/children").and_then(Value::as_array) {
                    if let Some(f) = find(c, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        find(self.manifest.pointer("/document/layers")?.as_array()?, id)
    }
}

fn write_media(path: &Path, raw: &[u8]) -> Result<()> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, raw)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn max_layer_id(doc: &Value) -> u64 {
    fn walk(layers: &[Value]) -> u64 {
        layers.iter().map(|l| l["id"].as_u64().unwrap_or(0).max(l.pointer("/content/children").and_then(Value::as_array).map(|c| walk(c)).unwrap_or(0))).max().unwrap_or(0)
    }
    doc.get("layers").and_then(Value::as_array).map(|l| walk(l)).unwrap_or(0)
}

/// Put an owner's tiles back into its surfaces.
fn restore_tiles(state: &mut Value, owner: &str, tiles: &BTreeMap<(String, String), Vec<Value>>) {
    for ((o, surface), list) in tiles.range((owner.to_string(), String::new())..) {
        if o != owner {
            break;
        }
        if let Some(s) = state.pointer_mut(surface).and_then(Value::as_object_mut) {
            s.insert("tiles".into(), Value::Array(list.clone()));
        }
    }
}

/// Tile and blob hashes a manifest names.
fn collect_hashes(v: &Value, tiles: &mut BTreeSet<String>, blobs: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            if let (Some(h), true) = (m.get("hash").and_then(Value::as_str), m.contains_key("tx")) {
                tiles.insert(h.to_string());
                return;
            }
            m.values().for_each(|x| collect_hashes(x, tiles, blobs));
        }
        Value::Array(a) => a.iter().for_each(|x| collect_hashes(x, tiles, blobs)),
        Value::String(s) if is_hash(s) => {
            blobs.insert(s.clone());
        }
        _ => {}
    }
}

impl Connector for PhotoConnector {
    fn info(&self) -> ConnectorInfo {
        let k = |kind: &str, lane: &str, mergeable: bool| KindInfo { kind: kind.into(), lane: lane.into(), schema: 1, mergeable };
        ConnectorInfo {
            app: "ubunye.photo".into(),
            contract: CONTRACT_VERSION,
            kinds: vec![
                k("photo.document", "layers", true),
                k("photo.layer", "layers", true),
                k("photo.raster", "retouch", true),
                k("photo.adjustment", "adjust", true),
                k("photo.text", "text", true),
                k("photo.tile.layers", "layers", false),
                k("photo.tile.retouch", "retouch", false),
                k("photo.tile.adjust", "adjust", false),
                k("photo.tile.text", "text", false),
            ],
        }
    }

    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        self.flush()?;
        self.load()?;
        Ok(self.items.clone())
    }

    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        self.ensure_loaded()?;
        match item {
            Some(i) => {
                ensure!(self.info().owns(&i.kind), "the Photo connector does not own kind {}", i.kind);
                for a in &i.assets {
                    ensure!(self.media_path(&a.hash).exists(), "media {} for {key} is not here yet; fetch it before applying", a.hash);
                }
                if self.items.get(key) == Some(i) {
                    return Ok(());
                }
                self.items.insert(key.to_string(), i.clone());
            }
            None => {
                if self.items.remove(key).is_none() {
                    return Ok(());
                }
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn describe(&self, before: Option<&Item>, after: &Item) -> Vec<String> {
        if after.kind.starts_with("photo.tile.") {
            return vec![if before.is_some() { "repainted one tile".into() } else { "painted a new tile".into() }];
        }
        kopano_core::jsonmerge::describe(before.map(|b| &b.state), &after.state)
    }
}

impl Drop for PhotoConnector {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl Harness for PhotoConnector {
    fn fresh(&self) -> Self {
        match PhotoConnector::headless_demo(&self.cli, &self.media) {
            Ok(c) => c,
            Err(e) => panic!("cannot start a fresh Photo engine for conformance: {e:#}"),
        }
    }
    fn set_number(&mut self, key: &str, path: &str, value: f64) -> Result<()> {
        let uid = key.strip_prefix("layer/").context("not a layer key")?;
        let id = self.layer_id(uid)?;
        // Through Photo's own commands, exactly as a user's slider would.
        match path.split('.').collect::<Vec<_>>().as_slice() {
            ["content", "adjustment", _kind, field] => self.t.exec("layer.setAdjustment", json!({"layer": id, *field: value}))?,
            [field] => self.t.exec("layer.setProps", json!({"layer": id, *field: value}))?,
            _ => bail!("no Photo command sets {path}"),
        };
        self.loaded = false;
        Ok(())
    }
    fn get_number(&mut self, key: &str, path: &str) -> Result<f64> {
        let uid = key.strip_prefix("layer/").context("not a layer key")?;
        let id = self.layer_id(uid)?;
        self.load()?;
        let l = self.local_layer(id).with_context(|| format!("Photo has no layer {id}"))?;
        l.pointer(&format!("/{}", path.replace('.', "/"))).and_then(Value::as_f64).with_context(|| format!("no number at {path}"))
    }
    fn sample(&self) -> (String, String, String) {
        // The demo's Exposure layer; exposure and gamma are independent sliders.
        (self.sample_key.clone(), "content.adjustment.Exposure.exposure".into(), "content.adjustment.Exposure.gamma".into())
    }
}
