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
}

impl LightConnector {
    pub fn headless(cli: &str) -> Result<Self> {
        Ok(Self { t: Transport::headless_demo(cli)?, cli: cli.into(), ids: BTreeMap::new(), hashes: BTreeMap::new() })
    }
    pub fn live(port: u16) -> Result<Self> {
        Ok(Self { t: Transport::control(port)?, cli: String::new(), ids: BTreeMap::new(), hashes: BTreeMap::new() })
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

    fn id_for(&mut self, key: &str) -> Result<u64> {
        let base = key.rsplit_once('#').map(|(b, _)| b).unwrap_or(key);
        if !self.ids.contains_key(base) {
            self.snapshot()?;
        }
        self.ids.get(base).copied().with_context(|| format!("Light has no photo {base}"))
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
        let mut out = BTreeMap::new();
        self.ids.clear();
        for id in ids {
            let p = self.inspect(id)?;
            let key = Self::photo_key(&p);
            self.ids.insert(key.clone(), id);
            let assets = self.original(&p)?;
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
