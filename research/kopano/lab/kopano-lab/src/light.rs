//! Kopano connector for Ubunye Light: the first real app on the neutral contract.
//!
//! It drives Light through Light's own automation surfaces, so Light needs no changes:
//! - `Transport::Mcp`: `lightcraft-cli mcp` headless engine over stdio (tests, no window), or
//! - `Transport::Control`: the running desktop app's control port (`lightcraft --control PORT`).
//!
//! Each Light photo becomes three Kopano items, one per lane, so permissions can differ:
//! `<photo>#develop` (develop settings), `<photo>#selects` (rating, flag, label),
//! `<photo>#meta` (title, caption, keywords, ...). The photo key is Light's content hash when it
//! has one, else file name + capture time; it never uses Light's local row id.

use anyhow::{bail, Context, Result};
use kopano_core::conformance::Harness;
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::project::{AssetRef, Item};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const META: [(&str, &str); 10] = [
    ("title", "title"),
    ("caption", "caption"),
    ("alt_text", "altText"),
    ("keywords", "keywords"),
    ("creator", "creator"),
    ("copyright", "copyright"),
    ("location", "location"),
    ("city", "city"),
    ("state", "state"),
    ("country", "country"),
];

pub enum Transport {
    Mcp { child: Child, stdin: ChildStdin, stdout: BufReader<ChildStdout>, next: u64 },
    Control { reader: BufReader<std::net::TcpStream>, writer: std::net::TcpStream, next: u64 },
}

impl Transport {
    /// Headless Light engine with the procedural demo library, in memory only.
    pub fn headless_demo(cli: &str) -> Result<Self> {
        let mut child = Command::new(cli)
            .args(["mcp", "--demo"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("cannot start {cli}"))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("no stdout")?);
        let mut t = Transport::Mcp { child, stdin, stdout, next: 0 };
        t.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "kopano", "version": "0"}}))?;
        if let Transport::Mcp { stdin, .. } = &mut t {
            writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        }
        Ok(t)
    }

    pub fn control(port: u16) -> Result<Self> {
        let s = std::net::TcpStream::connect(("127.0.0.1", port)).with_context(|| format!("Ubunye Light is not listening on control port {port}"))?;
        Ok(Transport::Control { reader: BufReader::new(s.try_clone()?), writer: s, next: 0 })
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let Transport::Mcp { stdin, stdout, next, .. } = self else { bail!("not an MCP transport") };
        *next += 1;
        let id = *next;
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        stdin.flush()?;
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line)? == 0 {
                bail!("Light's engine stopped");
            }
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(e) = v.get("error") {
                bail!("Light refused {method}: {e}");
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Run one engine command by id.
    pub fn exec(&mut self, command: &str, params: Value) -> Result<Value> {
        match self {
            Transport::Mcp { .. } => {
                let r = self.rpc("tools/call", json!({"name": "run_command", "arguments": {"command": command, "params": params}}))?;
                let text = r.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("null");
                if r.get("isError") == Some(&Value::Bool(true)) {
                    bail!("Light could not run {command}: {text}");
                }
                Ok(serde_json::from_str(text).unwrap_or(Value::String(text.into())))
            }
            Transport::Control { reader, writer, next } => {
                *next += 1;
                writeln!(writer, "{}", json!({"id": *next, "method": "engine.execute", "params": {"command": command, "params": params}}))?;
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let v: Value = serde_json::from_str(&line)?;
                if v.get("ok") != Some(&Value::Bool(true)) {
                    bail!("Light could not run {command}: {}", v.get("error").and_then(Value::as_str).unwrap_or("?"));
                }
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if let Transport::Mcp { child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct LightConnector {
    pub t: Transport,
    cli: String,
    /// photo key -> Light's local id, refreshed on every snapshot.
    ids: BTreeMap<String, u64>,
    /// BLAKE3 of originals by (path, size): hashing is the expensive part, done once per file.
    hashes: BTreeMap<(String, u64), String>,
    /// Photos shown from a preview because the original is on a teammate's computer (kopanod
    /// media): the preview file's path in Light -> the project's photo key and original.
    pub previews: BTreeMap<String, PreviewAlias>,
    /// Where kopanod keeps preview files: a photo from there that has no alias is never
    /// published as a new photo of the project.
    pub preview_dir: Option<String>,
    /// photo key -> Light id of the preview photo standing in for it (also when hidden because
    /// the original arrived and was imported beside it).
    alias_ids: BTreeMap<String, u64>,
}

/// A preview standing in for an original (kopanod media). The key is the ORIGINAL's photo key
/// (Light's content hash of the original file), so edits made on the preview apply to the same
/// photo on every device; the assets are the original's, so the project never learns the
/// preview's hash as if it were a new original.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PreviewAlias {
    pub key: String,
    pub assets: Vec<AssetRef>,
}

/// One way to spell a path, so Light's stored path and ours compare equal.
pub fn norm_path(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\").trim_start_matches(r"\\?\").to_lowercase()
    } else {
        p.to_string()
    }
}

/// How each photo Light reports maps to the project: its key and, for a preview, the alias.
/// `None`: kopanod must not publish this photo (a preview file without an alias, or a preview
/// whose original now sits beside it in Light: the original wins).
pub fn resolve_keys<'a>(photos: &[Value], previews: &'a BTreeMap<String, PreviewAlias>, preview_dir: Option<&str>) -> Vec<Option<(String, Option<&'a PreviewAlias>)>> {
    let path_of = |p: &Value| p.pointer("/source/path").and_then(Value::as_str).map(norm_path);
    let copy = |p: &Value| p.get("copy_of").is_some_and(|c| !c.is_null()).then(|| format!("#copy:{}", p.get("copy_name").and_then(Value::as_str).unwrap_or("")));
    let mut out: Vec<Option<(String, Option<&PreviewAlias>)>> = photos
        .iter()
        .map(|p| match path_of(p) {
            Some(path) if previews.contains_key(&path) => {
                let a = &previews[&path];
                Some((format!("{}{}", a.key, copy(p).unwrap_or_default()), Some(a)))
            }
            Some(path) if preview_dir.is_some_and(|d| path.starts_with(&norm_path(d))) => None,
            _ => Some((LightConnector::photo_key(p), None)),
        })
        .collect();
    let native: std::collections::BTreeSet<String> = out.iter().flatten().filter(|(_, a)| a.is_none()).map(|(k, _)| k.clone()).collect();
    for o in out.iter_mut() {
        if matches!(o, Some((k, Some(_))) if native.contains(k)) {
            *o = None;
        }
    }
    out
}

impl LightConnector {
    pub fn headless(cli: &str) -> Result<Self> {
        Ok(Self::with(Transport::headless_demo(cli)?, cli))
    }
    pub fn live(port: u16) -> Result<Self> {
        Ok(Self::with(Transport::control(port)?, ""))
    }
    fn with(t: Transport, cli: &str) -> Self {
        Self { t, cli: cli.into(), ids: BTreeMap::new(), hashes: BTreeMap::new(), previews: BTreeMap::new(), preview_dir: None, alias_ids: BTreeMap::new() }
    }

    /// Light shows this photo (natively or from a preview), as of the last snapshot.
    pub fn has_photo(&self, key: &str) -> bool {
        self.ids.contains_key(key.rsplit_once('#').map(|(b, _)| b).unwrap_or(key))
    }
    /// Light's own id of the preview photo standing in for `key`, as of the last snapshot.
    pub fn alias_id(&self, key: &str) -> Option<u64> {
        self.alias_ids.get(key).copied()
    }
    /// Light shows this photo from its original file (not from a preview).
    pub fn has_original(&self, key: &str) -> bool {
        self.has_photo(key) && self.ids.get(key) != self.alias_ids.get(key)
    }
    /// The file on this computer whose BLAKE3 is `hash`, if Light has imported it.
    pub fn original_path(&self, hash: &str) -> Option<String> {
        self.hashes.iter().find(|(_, h)| h.as_str() == hash).map(|((p, _), _)| p.clone()).filter(|p| std::path::Path::new(p).is_file())
    }
    /// Add files to Light's library in place (no copy). Light imports on a worker thread: the
    /// photos show up in a later snapshot.
    pub fn import_files(&mut self, paths: &[String]) -> Result<Value> {
        self.t.exec("library.import", json!({"paths": paths, "mode": "add"}))
    }
    /// Take a photo out of Light's library for good (a preview replaced by its original).
    pub fn remove_photo(&mut self, id: u64) -> Result<()> {
        self.t.exec("photo.deletePermanently", json!({"ids": [id]}))?;
        Ok(())
    }

    fn photo_key(p: &Value) -> String {
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut key = match p.get("content_hash").and_then(Value::as_str) {
            Some(h) if !h.is_empty() => h.to_string(),
            _ => format!("{}|{}", s("file_name"), s("captured")),
        };
        if p.get("copy_of").is_some_and(|c| !c.is_null()) {
            key.push_str(&format!("#copy:{}", s("copy_name")));
        }
        key
    }

    fn original(&mut self, p: &Value) -> Result<Vec<AssetRef>> {
        let Some(path) = p.pointer("/source/path").and_then(Value::as_str) else { return Ok(vec![]) };
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let k = (path.to_string(), size);
        if !self.hashes.contains_key(&k) {
            let mut f = std::fs::File::open(path)?;
            let mut h = blake3::Hasher::new();
            std::io::copy(&mut f, &mut h)?;
            self.hashes.insert(k.clone(), h.finalize().to_hex().to_string());
        }
        Ok(vec![AssetRef { hash: self.hashes[&k].clone(), size, role: "original".into() }])
    }

    fn preview_alias_of(&self, p: &Value) -> Option<&PreviewAlias> {
        p.pointer("/source/path").and_then(Value::as_str).and_then(|path| self.previews.get(&norm_path(path)))
    }

    fn id_for(&mut self, key: &str) -> Result<u64> {
        let base = key.rsplit_once('#').map(|(b, _)| b).unwrap_or(key);
        if !self.ids.contains_key(base) {
            self.snapshot()?;
        }
        self.ids.get(base).copied().with_context(|| format!("Light has no photo {base}"))
    }

    fn all_ids(&mut self) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        loop {
            let page = self.t.exec("catalog.query", json!({"filter": {}, "offset": ids.len(), "limit": 500}))?;
            let photos = page.get("photos").and_then(Value::as_array).cloned().unwrap_or_default();
            ids.extend(photos.iter().filter_map(|p| p.get("id").and_then(Value::as_u64)));
            let total = page.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
            if photos.is_empty() || ids.len() >= total {
                break;
            }
        }
        Ok(ids)
    }

    fn inspect(&mut self, id: u64) -> Result<Value> {
        self.t.exec("photo.inspect", json!({"id": id}))
    }
}

impl Connector for LightConnector {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            app: "ubunye.light".into(),
            contract: CONTRACT_VERSION,
            kinds: vec![
                KindInfo { kind: "light.develop".into(), lane: "develop".into(), schema: 1, mergeable: true },
                KindInfo { kind: "light.selects".into(), lane: "selects".into(), schema: 1, mergeable: true },
                KindInfo { kind: "light.meta".into(), lane: "meta".into(), schema: 1, mergeable: true },
            ],
        }
    }

    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        // S3: Light builds with `photo.inspectMany` answer for the whole library in one request
        // (one UI frame); older builds are asked photo by photo.
        let photos = match self.t.exec("photo.inspectMany", json!({})) {
            Ok(Value::Array(a)) => a,
            _ => {
                let ids = self.all_ids()?;
                let mut v = Vec::with_capacity(ids.len());
                for id in ids {
                    v.push(self.inspect(id)?);
                }
                v
            }
        };
        let mut out = BTreeMap::new();
        self.ids.clear();
        self.alias_ids.clear();
        let keys: Vec<_> = resolve_keys(&photos, &self.previews, self.preview_dir.as_deref()).into_iter().map(|k| k.map(|(k, a)| (k, a.cloned()))).collect();
        for (p, k) in photos.iter().zip(keys) {
            let Some(id) = p.get("id").and_then(Value::as_u64) else { continue };
            if let Some(a) = self.preview_alias_of(p) {
                self.alias_ids.insert(a.key.clone(), id);
            }
            let Some((key, alias)) = k else { continue };
            self.ids.insert(key.clone(), id);
            let assets = match alias {
                Some(a) => a.assets,
                None => self.original(p)?,
            };
            let mut meta = serde_json::Map::new();
            for (f, _) in META {
                if let Some(v) = p.pointer(&format!("/meta/{f}")) {
                    meta.insert(f.into(), v.clone());
                }
            }
            out.insert(format!("{key}#develop"), Item { kind: "light.develop".into(), lane: "develop".into(), schema: 1, state: p.get("develop").cloned().unwrap_or(Value::Null), assets });
            out.insert(
                format!("{key}#selects"),
                Item { kind: "light.selects".into(), lane: "selects".into(), schema: 1, state: json!({"rating": p.get("rating"), "flag": p.get("flag"), "label": p.get("label")}), assets: vec![] },
            );
            out.insert(format!("{key}#meta"), Item { kind: "light.meta".into(), lane: "meta".into(), schema: 1, state: Value::Object(meta), assets: vec![] });
        }
        Ok(out)
    }

    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        let Some(item) = item else { bail!("removing photos through Kopano is not supported yet (key {key})") };
        let id = self.id_for(key)?;
        let ids = json!([id]);
        let current = self.inspect(id)?;
        match item.kind.as_str() {
            "light.develop" => {
                if current.get("develop") == Some(&item.state) {
                    return Ok(());
                }
                // develop.merge merges key by key; a section absent from the target would survive.
                let stale = match (current.get("develop").and_then(Value::as_object), item.state.as_object()) {
                    (Some(c), Some(t)) => c.keys().any(|k| !t.contains_key(k)),
                    _ => false,
                };
                if stale {
                    self.t.exec("develop.reset", json!({"ids": ids}))?;
                }
                self.t.exec("library.select", json!({"ids": ids, "active": id}))?;
                self.t.exec("develop.merge", json!({"settings": item.state, "label": "Kopano"}))?;
            }
            "light.selects" => {
                let s = &item.state;
                if current.get("rating") != s.get("rating") {
                    self.t.exec("photo.rate", json!({"rating": s.get("rating").and_then(Value::as_u64).unwrap_or(0), "ids": ids}))?;
                }
                if current.get("flag") != s.get("flag") {
                    self.t.exec("photo.flag", json!({"flag": s.get("flag").and_then(Value::as_str).unwrap_or("none"), "ids": ids}))?;
                }
                if current.get("label") != s.get("label") {
                    self.t.exec("photo.label", json!({"label": s.get("label").and_then(Value::as_str).unwrap_or("none"), "ids": ids}))?;
                }
            }
            "light.meta" => {
                let mut p = serde_json::Map::new();
                p.insert("ids".into(), ids);
                for (from, to) in META {
                    if let Some(v) = item.state.get(from) {
                        p.insert(to.into(), v.clone());
                    }
                }
                self.t.exec("photo.setMeta", Value::Object(p))?;
            }
            other => bail!("the Light connector does not own kind {other}"),
        }
        Ok(())
    }
}

impl Harness for LightConnector {
    fn fresh(&self) -> Self {
        // Conformance needs identical fresh instances: every headless demo library is the same.
        match LightConnector::headless(&self.cli) {
            Ok(c) => c,
            Err(e) => panic!("cannot start a fresh Light engine for conformance: {e:#}"),
        }
    }
    fn set_number(&mut self, key: &str, path: &str, value: f64) -> Result<()> {
        let id = self.id_for(key)?;
        // Through Light's own control, exactly as a user's slider would.
        self.t.exec("develop.set", json!({"control": path, "value": value, "ids": [id]}))?;
        Ok(())
    }
    fn get_number(&mut self, key: &str, path: &str) -> Result<f64> {
        let id = self.id_for(key)?;
        let p = self.inspect(id)?;
        p.pointer(&format!("/develop/{}", path.replace('.', "/"))).and_then(Value::as_f64).with_context(|| format!("no number at {path}"))
    }
    fn sample(&self) -> (String, String, String) {
        // Demo photo "Alpenglow" (LC01347.JPG); exposure and contrast are independent sliders.
        ("LC01347.JPG|2026-09-20T05:40:00#develop".into(), "light.exposure".into(), "light.contrast".into())
    }
}

/// A preview of an original that Light on another computer can import and edit (kopanod media).
///
/// Light's own rendering of the original at its as-imported settings, as a 2560 px JPEG: about
/// 0.6-0.7 MB for the owner's 24 MP Sony ARWs (49 MB). Sharpening and colour noise reduction are
/// left out because the receiving Light applies the project's own values again, and the
/// orientation stays as shot so the project's orientation setting turns it the same way there.
/// Measured against the RAW in the same Light (1280 px, mean difference out of 255): as shot
/// 0.8, contrast +50 1.0, white balance 4000 K 2.0-2.3, but exposure +-2 15-36 and
/// highlights/shadows 27-31: a picture to work on, not to judge tones on. The fix needs Light to
/// take its own Smart Preview in (E13: within 0.5).
pub fn make_preview(cli: &str, original: &std::path::Path, out: &std::path::Path) -> Result<()> {
    let settings = out.with_extension("settings.json");
    std::fs::write(&settings, json!({"detail": {"sharpen_amount": 0.0, "nr_color": 0.0}, "orientation": "Normal"}).to_string())?;
    let r = Command::new(cli)
        .arg("render")
        .arg(original)
        .arg("-o")
        .arg(out)
        .args(["--size", "2560", "--quality", "90", "--settings"])
        .arg(&settings)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("cannot start {cli}"));
    let _ = std::fs::remove_file(&settings);
    let r = r?;
    if !r.status.success() || !out.is_file() {
        bail!("Light could not make a preview of {}: {}", original.display(), String::from_utf8_lossy(&r.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: u64, path: &str, hash: &str, copy: Option<&str>) -> Value {
        json!({"id": id, "content_hash": hash, "file_name": path.rsplit(['/', '\\']).next(), "source": {"type": "file", "path": path},
            "copy_of": copy.map(|_| 1), "copy_name": copy})
    }
    fn alias(key: &str) -> PreviewAlias {
        PreviewAlias { key: key.into(), assets: vec![AssetRef { hash: "b3".into(), size: 49_000_000, role: "original".into() }] }
    }

    #[test]
    fn a_preview_carries_the_originals_key() {
        let dir = if cfg!(windows) { r"C:\K\media\previews" } else { "/k/media/previews" };
        let file = format!("{dir}{}DSC08379.jpg", std::path::MAIN_SEPARATOR);
        let previews = BTreeMap::from([(norm_path(&file), alias("e21e378d"))]);
        let photos = vec![photo(1, &file, "750a53e9", None), photo(2, &file, "750a53e9", Some("B&W"))];
        let k = resolve_keys(&photos, &previews, Some(dir));
        assert_eq!(k[0].as_ref().map(|(k, a)| (k.as_str(), a.is_some())), Some(("e21e378d", true)), "the preview's own hash is never the key");
        assert_eq!(k[1].as_ref().map(|(k, _)| k.as_str()), Some("e21e378d#copy:B&W"), "a virtual copy of a preview keeps the original's key");
    }

    #[test]
    fn a_preview_file_without_an_alias_is_never_published() {
        let dir = if cfg!(windows) { r"C:\K\media\previews" } else { "/k/media/previews" };
        let stray = format!("{dir}{}x.jpg", std::path::MAIN_SEPARATOR);
        let none = BTreeMap::new();
        let k = resolve_keys(&[photo(1, &stray, "aa", None), photo(2, "/home/me/own.jpg", "bb", None)], &none, Some(dir));
        assert!(k[0].is_none());
        assert_eq!(k[1].as_ref().map(|(k, _)| k.as_str()), Some("bb"));
    }

    #[test]
    fn the_original_wins_over_its_preview() {
        let previews = BTreeMap::from([(norm_path("/k/p/a.jpg"), alias("e21e378d"))]);
        let photos = vec![photo(1, "/k/p/a.jpg", "750a", None), photo(2, "/k/o/DSC08379.ARW", "e21e378d", None)];
        let k = resolve_keys(&photos, &previews, Some("/k/p"));
        assert!(k[0].is_none(), "the preview stands down once the original is in Light");
        assert_eq!(k[1].as_ref().map(|(k, a)| (k.as_str(), a.is_none())), Some(("e21e378d", true)));
    }

    #[test]
    fn paths_compare_however_they_are_spelled() {
        if cfg!(windows) {
            assert_eq!(norm_path("T:/Kopano/A.jpg"), norm_path(r"t:\kopano\a.JPG"));
            assert_eq!(norm_path(r"\\?\T:\a.jpg"), norm_path("T:/a.jpg"));
        }
    }
}
