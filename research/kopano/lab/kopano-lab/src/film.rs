//! Kopano connector for Ubunye Film (the video editor).
//!
//! It drives Film through Film's own automation surfaces:
//! - `Transport::Mcp`: `filmcraft-cli mcp` headless engine over stdio (tests, no window), or
//! - `Transport::Control`: the running desktop app's control port (`filmcraft --control PORT`).
//!
//! Film (branch `feature/kopano-stable-ids`) gives two things this needs:
//! - ids unique across computers: each session allocates from its own id block, so two editors
//!   never both make "clip 7";
//! - `kopano.items` / `kopano.apply`: the timeline split into lanes, lossless both ways.
//!
//! Items, per sequence `S` (a stable project id):
//! - `film/S/clip/C#edit`  kind `film.clip`  lane `edit`:  track, cut [start, duration, sourceIn],
//!   speed, name, label, links, Motion/Opacity and other non-colour effects. Media is named by
//!   the BLAKE3 hash of the original (`media` + an asset ref), never by Film's local item id.
//! - `film/S/clip/C#grade` kind `film.grade` lane `grade`: colour effects, keyframes in source
//!   time (a trim never moves a grade off its frames).
//! - `film/S/clip/C#sound` kind `film.sound` lane `sound`: gain, Essential Sound, audio effects.
//! - `film/S/track/T#edit` kind `film.track` lane `edit`; `film/S/track/T#mix` kind `film.mix`
//!   lane `sound` (volume, pan, mute, solo, inserts, audio fades).
//!
//! What merges: every kind is merged setting by setting. A trim is one setting (`cut` is an
//! array, and arrays never merge inside), so two trims of one clip are a clash for a person,
//! while a trim and a grade, or a grade and a level change, both survive.

use anyhow::{bail, Context, Result};
use kopano_core::conformance::Harness;
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::project::{AssetRef, Item};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub enum Transport {
    Mcp { child: Child, stdin: ChildStdin, stdout: BufReader<ChildStdout>, next: u64 },
    Control { reader: BufReader<std::net::TcpStream>, writer: std::net::TcpStream, next: u64 },
}

impl Transport {
    /// Headless Film engine: `--demo` or `--project <file>`, in memory only.
    pub fn headless(cli: &str, project: Option<&str>) -> Result<Self> {
        let mut cmd = Command::new(cli);
        cmd.arg("mcp");
        match project {
            Some(p) => cmd.args(["--project", p]),
            None => cmd.arg("--demo"),
        };
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().with_context(|| format!("cannot start {cli}"))?;
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
        let s = std::net::TcpStream::connect(("127.0.0.1", port)).with_context(|| format!("Ubunye Film is not listening on control port {port}"))?;
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
                bail!("Film's engine stopped");
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(e) = v.get("error") {
                bail!("Film refused {method}: {e}");
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Run one engine command by id.
    pub fn exec(&mut self, command: &str, params: Value) -> Result<Value> {
        match self {
            Transport::Mcp { .. } => {
                let r = self.rpc("tools/call", json!({"name": "command_run", "arguments": {"id": command, "params": params}}))?;
                let text = r.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("null");
                if r.get("isError") == Some(&Value::Bool(true)) {
                    bail!("Film could not run {command}: {text}");
                }
                if let Some(sc) = r.get("structuredContent") {
                    return Ok(sc.clone());
                }
                Ok(serde_json::from_str(text).unwrap_or(Value::String(text.into())))
            }
            Transport::Control { .. } => self.call("engine.execute", json!({"command": command, "params": params})),
        }
    }

    /// A control-channel method (desktop app only), e.g. `ui.screenshot`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let Transport::Control { reader, writer, next } = self else { bail!("{method} needs the desktop app's control port") };
        *next += 1;
        writeln!(writer, "{}", json!({"id": *next, "method": method, "params": params}))?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("Film closed the control connection");
        }
        let v: Value = serde_json::from_str(&line)?;
        if v.get("ok") != Some(&Value::Bool(true)) {
            bail!("Film could not run {method}: {}", v.get("error").and_then(Value::as_str).unwrap_or("?"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
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

/// How fresh instances are made for the conformance kit.
#[derive(Clone)]
pub enum Fresh {
    /// A new headless engine per instance (`project` None = Film's demo project).
    Headless { cli: String, project: Option<String> },
    /// Running windows: each fresh instance reopens `project` in the next window in turn, so the
    /// two instances of a merge test are two different windows.
    Windows { ports: Vec<u16>, project: String },
}

pub struct FilmConnector {
    pub t: Transport,
    fresh: Fresh,
    turn: Cell<usize>,
    /// BLAKE3 of originals by (path, size): hashing is the expensive part, done once per file.
    hashes: BTreeMap<(String, u64), String>,
    /// hash -> Film's local media item id, refreshed on every snapshot.
    media_items: BTreeMap<String, u64>,
    /// Local files by BLAKE3, for media a teammate used that this computer has not imported
    /// yet (kopanod's media store hands these over).
    pub media_files: BTreeMap<String, String>,
    /// Parts of clips whose `edit` part has not arrived yet.
    pending: BTreeMap<(u64, u64), BTreeMap<String, Value>>,
    /// The grade key of the first video clip, for the conformance merge tests.
    sample_key: String,
}

fn kinds() -> Vec<KindInfo> {
    let k = |kind: &str, lane: &str| KindInfo { kind: kind.into(), lane: lane.into(), schema: 1, mergeable: true };
    vec![k("film.clip", "edit"), k("film.grade", "grade"), k("film.sound", "sound"), k("film.track", "edit"), k("film.mix", "sound")]
}

fn kind_of(part: &str, is_track: bool) -> Result<(&'static str, &'static str)> {
    Ok(match (is_track, part) {
        (false, "edit") => ("film.clip", "edit"),
        (false, "grade") => ("film.grade", "grade"),
        (false, "sound") => ("film.sound", "sound"),
        (true, "edit") => ("film.track", "edit"),
        (true, "mix") => ("film.mix", "sound"),
        _ => bail!("unknown part {part}"),
    })
}

/// `film/S/clip/C#part` -> (S, is_track, C, part)
fn parse_key(key: &str) -> Result<(u64, bool, u64, String)> {
    let (path, part) = key.split_once('#').with_context(|| format!("bad key {key}"))?;
    let bits: Vec<&str> = path.split('/').collect();
    let [app, seq, what, id] = bits[..] else { bail!("bad key {key}") };
    if app != "film" || !(what == "clip" || what == "track") {
        bail!("bad key {key}");
    }
    Ok((seq.parse()?, what == "track", id.parse()?, part.to_string()))
}

impl FilmConnector {
    pub fn headless(cli: &str, project: Option<&str>) -> Result<Self> {
        let fresh = Fresh::Headless { cli: cli.into(), project: project.map(String::from) };
        Self::with(Transport::headless(cli, project)?, fresh)
    }
    /// Connect to a running window. `fresh` is only used by the conformance kit.
    pub fn live(port: u16, fresh: Fresh) -> Result<Self> {
        Self::with(Transport::control(port)?, fresh)
    }
    fn with(t: Transport, fresh: Fresh) -> Result<Self> {
        let mut c = Self { t, fresh, turn: Cell::new(0), hashes: BTreeMap::new(), media_items: BTreeMap::new(), media_files: BTreeMap::new(), pending: BTreeMap::new(), sample_key: String::new() };
        c.find_sample()?;
        Ok(c)
    }

    /// Open a project file in the app (all earlier state in that window is replaced).
    pub fn open(&mut self, path: &str) -> Result<()> {
        self.t.exec("file.open", json!({"path": path}))?;
        self.pending.clear();
        self.find_sample()
    }

    /// The first clip on a video track of the first sequence that has one.
    fn find_sample(&mut self) -> Result<()> {
        let all = self.items_raw()?;
        for seq in all.get("sequences").and_then(Value::as_array).cloned().unwrap_or_default() {
            let first = seq.get("clips").and_then(Value::as_array).and_then(|cs| cs.iter().find(|c| c.get("kind") == Some(&json!("video"))).cloned());
            if let (Some(sid), Some(c)) = (seq.get("id").and_then(Value::as_u64), first) {
                self.sample_key = format!("film/{sid}/clip/{}#grade", c.get("id").and_then(Value::as_u64).unwrap_or(0));
                return Ok(());
            }
        }
        Ok(())
    }

    fn hash_file(&mut self, path: &str) -> Result<Option<(String, u64)>> {
        let Ok(meta) = std::fs::metadata(path) else { return Ok(None) };
        let k = (path.to_string(), meta.len());
        if !self.hashes.contains_key(&k) {
            let mut h = blake3::Hasher::new();
            std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
            self.hashes.insert(k.clone(), h.finalize().to_hex().to_string());
        }
        Ok(Some((self.hashes[&k].clone(), meta.len())))
    }

    /// Film item id -> (hash, size) for every media item backed by a readable file.
    fn media_table(&mut self, all: &Value) -> Result<BTreeMap<u64, (String, u64)>> {
        let mut out = BTreeMap::new();
        self.media_items.clear();
        for m in all.get("media").and_then(Value::as_array).cloned().unwrap_or_default() {
            let (Some(item), Some(path)) = (m.get("item").and_then(Value::as_u64), m.get("path").and_then(Value::as_str)) else { continue };
            if let Some((hash, size)) = self.hash_file(path)? {
                self.media_items.entry(hash.clone()).or_insert(item);
                self.media_files.entry(hash.clone()).or_insert_with(|| path.to_string());
                out.insert(item, (hash, size));
            }
        }
        Ok(out)
    }

    /// The local media item for `hash`, importing the file when this project lacks it.
    fn local_item(&mut self, hash: &str) -> Result<u64> {
        if let Some(i) = self.media_items.get(hash) {
            return Ok(*i);
        }
        let path = self.media_files.get(hash).cloned().with_context(|| format!("media {hash} is not on this computer yet"))?;
        let before: Vec<u64> = self.items_raw()?.get("media").and_then(Value::as_array).cloned().unwrap_or_default().iter().filter_map(|m| m.get("item").and_then(Value::as_u64)).collect();
        self.t.exec("file.import", json!({"paths": [path]}))?;
        let all = self.items_raw()?;
        self.media_table(&all)?;
        let id = self.media_items.get(hash).copied().filter(|i| !before.contains(i));
        id.or_else(|| self.media_items.get(hash).copied()).with_context(|| format!("importing {hash} did not add it"))
    }

    fn items_raw(&mut self) -> Result<Value> {
        self.t.exec("kopano.items", json!({}))
    }

    fn send(&mut self, seq: u64, is_track: bool, id: u64, parts: Map) -> Result<()> {
        let mut p = json!({"sequence": seq});
        p[if is_track { "track" } else { "clip" }] = json!(id);
        for (k, v) in parts {
            p[k] = v;
        }
        self.t.exec("kopano.apply", p)?;
        Ok(())
    }

    /// Film's id of the clip a Kopano key names, and its sequence.
    pub fn clip_of(key: &str) -> Result<(u64, u64)> {
        let (seq, is_track, id, _) = parse_key(key)?;
        if is_track {
            bail!("{key} is a track");
        }
        Ok((seq, id))
    }
}

type Map = serde_json::Map<String, Value>;

impl Connector for FilmConnector {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo { app: "ubunye.film".into(), contract: CONTRACT_VERSION, kinds: kinds() }
    }

    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        let all = self.items_raw()?;
        let media = self.media_table(&all)?;
        let mut out = BTreeMap::new();
        for seq in all.get("sequences").and_then(Value::as_array).cloned().unwrap_or_default() {
            let sid = seq.get("id").and_then(Value::as_u64).context("sequence without id")?;
            for t in seq.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default() {
                let tid = t.get("id").and_then(Value::as_u64).context("track without id")?;
                for part in ["edit", "mix"] {
                    let Some(state) = t.get(part).filter(|v| !v.is_null()).cloned() else { continue };
                    let (kind, lane) = kind_of(part, true)?;
                    out.insert(format!("film/{sid}/track/{tid}#{part}"), Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets: vec![] });
                }
            }
            for c in seq.get("clips").and_then(Value::as_array).cloned().unwrap_or_default() {
                let cid = c.get("id").and_then(Value::as_u64).context("clip without id")?;
                for part in ["edit", "grade", "sound"] {
                    let Some(mut state) = c.get(part).filter(|v| !v.is_null()).cloned() else { continue };
                    let mut assets = vec![];
                    if part == "edit" {
                        let item = state.get("item").and_then(Value::as_u64);
                        if let Some((hash, size)) = item.and_then(|i| media.get(&i)) {
                            if let Some(m) = state.as_object_mut() {
                                m.remove("item");
                                m.insert("media".into(), json!(hash));
                            }
                            assets.push(AssetRef { hash: hash.clone(), size: *size, role: "original".into() });
                        }
                    }
                    let (kind, lane) = kind_of(part, false)?;
                    out.insert(format!("film/{sid}/clip/{cid}#{part}"), Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets });
                }
            }
        }
        Ok(out)
    }

    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        let (seq, is_track, id, part) = parse_key(key)?;
        let Some(item) = item else {
            // A removed clip or track goes with its edit part; other parts reset to empty.
            match (is_track, part.as_str()) {
                (_, "edit") => {
                    let mut p = json!({"sequence": seq, "remove": true});
                    p[if is_track { "track" } else { "clip" }] = json!(id);
                    self.t.exec("kopano.apply", p)?;
                }
                (false, "grade" | "sound") => {
                    let exists = self.snapshot()?.contains_key(&format!("film/{seq}/clip/{id}#edit"));
                    if exists {
                        let mut parts = Map::new();
                        parts.insert(part.clone(), json!({"fx": {}}));
                        self.send(seq, false, id, parts)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        };
        let (kind, _) = kind_of(&part, is_track)?;
        if item.kind != kind {
            bail!("{key} is {kind}, not {}", item.kind);
        }
        let mut state = item.state.clone();
        if !is_track && part == "edit" {
            if let Some(hash) = state.get("media").and_then(Value::as_str).map(String::from) {
                let local = self.local_item(&hash)?;
                if let Some(m) = state.as_object_mut() {
                    m.remove("media");
                    m.insert("item".into(), json!(local));
                }
            }
        }
        let mut parts = Map::new();
        parts.insert(part.clone(), state);
        if !is_track {
            if part == "edit" {
                parts.extend(self.pending.remove(&(seq, id)).unwrap_or_default());
            } else {
                // a new clip's grade or sound can arrive before its edit part: hold it until then
                let known = self.t.exec("kopano.items", json!({"sequence": seq}))?;
                let exists = known.pointer("/sequences/0/clips").and_then(Value::as_array).is_some_and(|cs| cs.iter().any(|c| c.get("id") == Some(&json!(id))));
                if !exists {
                    self.pending.entry((seq, id)).or_default().insert(part, item.state.clone());
                    return Ok(());
                }
            }
        }
        self.send(seq, is_track, id, parts)
    }

    fn describe(&self, before: Option<&Item>, after: &Item) -> Vec<String> {
        let mut out = vec![];
        // effect settings in plain words: "lumetri exposure 0.00 -> 0.90"
        let (bf, af) = (before.and_then(|b| b.state.get("fx")).cloned().unwrap_or(json!({})), after.state.get("fx").cloned().unwrap_or(json!({})));
        if let Some(fx) = af.as_object() {
            for (name, e) in fx {
                let Some(old) = bf.get(name) else {
                    out.push(format!("{name} added"));
                    continue;
                };
                for (p, v) in e.get("params").and_then(Value::as_object).into_iter().flatten() {
                    let was = old.pointer(&format!("/params/{p}"));
                    if was != Some(v) {
                        let num = |x: Option<&Value>| x.and_then(|x| x.pointer("/value/Float")).and_then(Value::as_f64);
                        match (num(was), num(Some(v))) {
                            (Some(a), Some(b)) => out.push(format!("{name} {p} {a:.2} -> {b:.2}")),
                            _ => out.push(format!("{name} {p} changed")),
                        }
                    }
                }
            }
        }
        if let Some(fx) = bf.as_object() {
            out.extend(fx.keys().filter(|k| af.get(*k).is_none()).map(|k| format!("{k} removed")));
        }
        let strip = |v: Option<&Value>| v.map(|v| {
            let mut v = v.clone();
            if let Some(m) = v.as_object_mut() {
                m.remove("fx");
            }
            v
        });
        let rest = strip(Some(&after.state)).unwrap_or(Value::Null);
        out.extend(kopano_core::jsonmerge::describe(strip(before.map(|b| &b.state)).as_ref(), &rest));
        // a trim in plain words
        if let (Some(b), Some(a)) = (before.and_then(|b| b.state.get("cut")), after.state.get("cut")) {
            if a != b {
                out.retain(|l| !l.starts_with("Cut"));
                let tps = 254_016_000_000f64;
                let n = |v: &Value, i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0) / tps;
                out.push(format!("trimmed: starts {:.2}s, {:.2}s long, from source {:.2}s (was {:.2}s, {:.2}s, {:.2}s)", n(a, 0), n(a, 1), n(a, 2), n(b, 0), n(b, 1), n(b, 2)));
            }
        }
        out
    }
}

impl Harness for FilmConnector {
    fn fresh(&self) -> Self {
        let made = match &self.fresh {
            Fresh::Headless { cli, project } => FilmConnector::headless(cli, project.as_deref()).and_then(|mut c| {
                // the demo has no colour effect: give every fresh copy the same Lumetri
                if project.is_none() {
                    let (_, clip) = FilmConnector::clip_of(&c.sample().0)?;
                    c.t.exec("effects.apply", json!({"clips": [clip], "effect": "lumetri"}))?;
                }
                Ok(c)
            }),
            Fresh::Windows { ports, project } => {
                let i = self.turn.get();
                self.turn.set(i + 1);
                let port = ports[i % ports.len()];
                FilmConnector::live(port, self.fresh.clone()).and_then(|mut c| {
                    c.turn.set(i + 1);
                    c.open(project)?;
                    Ok(c)
                })
            }
        };
        match made {
            Ok(c) => c,
            Err(e) => panic!("cannot make a fresh Film instance for conformance: {e:#}"),
        }
    }
    fn set_number(&mut self, key: &str, path: &str, value: f64) -> Result<()> {
        let (_, clip) = Self::clip_of(key)?;
        // Through Film's own command, exactly as a slider in the Lumetri panel would.
        let param = path.strip_prefix("fx.lumetri.params.").and_then(|p| p.strip_suffix(".value.Float")).with_context(|| format!("not a Lumetri setting: {path}"))?;
        self.t.exec("effects.setParam", json!({"clip": clip, "effect": "lumetri", "param": param, "value": value}))?;
        Ok(())
    }
    fn get_number(&mut self, key: &str, path: &str) -> Result<f64> {
        let snap = self.snapshot()?;
        let item = snap.get(key).with_context(|| format!("no item {key}"))?;
        item.state.pointer(&format!("/{}", path.replace('.', "/"))).and_then(Value::as_f64).with_context(|| format!("no number at {path}"))
    }
    fn sample(&self) -> (String, String, String) {
        // Exposure and contrast: two independent Lumetri sliders of the first video clip.
        (self.sample_key.clone(), "fx.lumetri.params.exposure.value.Float".into(), "fx.lumetri.params.contrast.value.Float".into())
    }
}
