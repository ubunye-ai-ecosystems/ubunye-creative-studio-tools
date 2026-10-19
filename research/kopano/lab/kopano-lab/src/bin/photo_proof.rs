//! Real-app proof for the Ubunye Photo connector: two Photo windows (two "computers"), edits made
//! in one reach the other through Kopano versions, and we count the bytes each edit moves.
//!
//! Both windows are driven only through Photo's control channel (no OS-level input). Each window
//! has its own automation folder, its own media folder and its own Kopano store; media moves
//! between the media folders only when a version names it (standing in for the node's fetch).
//!
//! Usage: photo_proof --exe <photocraft.exe> --doc <owner.pcraft> --out <dir> [--ports 18301,18302]
//! Writes <out>/{a,b}/<step>.png screenshots and <out>/measurements.json.

use anyhow::{bail, Context, Result};
use kopano_core::connector::Connector;
use kopano_core::identity::{DeviceKey, Provider};
use kopano_core::project::{Item, Member, Project, Role};
use kopano_core::repo::{copy_version_closure, Integrated, Repo};
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_lab::photo::PhotoConnector;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Window {
    child: Child,
    c: PhotoConnector,
    media: PathBuf,
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.c.t.call("app.quit", json!({}));
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn launch(exe: &str, out: &Path, name: &str, port: u16, doc: &Path) -> Result<Window> {
    let root = out.join(name);
    let media = out.join(format!("media-{name}"));
    let _ = std::fs::remove_dir_all(&media);
    std::fs::create_dir_all(&root)?;
    std::fs::copy(doc, root.join("owner.pcraft"))?;
    let token = out.join(format!("{name}.token"));
    let mut child = Command::new(exe)
        .args(["--control", &port.to_string(), "--control-token-file"])
        .arg(&token)
        .arg("--automation-read-root")
        .arg(&root)
        .arg("--automation-write-root")
        .arg(&root)
        .spawn()
        .with_context(|| format!("cannot start {exe}"))?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let c = loop {
        if let Ok(t) = std::fs::read_to_string(&token) {
            if let Ok(c) = PhotoConnector::live(port, &t, &media) {
                break c;
            }
        }
        if let Ok(Some(s)) = child.try_wait() {
            bail!("Photo window {name} exited early: {s}");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            bail!("Photo window {name} did not open its control port {port}");
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let mut w = Window { child, c, media };
    w.c.t.call("ui.resize", json!({"width": 1200, "height": 780}))?;
    let r = w.c.t.call("app.open", json!({"path": "owner.pcraft"}))?;
    println!("[{name}] opened {}", r["name"]);
    // Opening may finish as a background job; wait until the document is there.
    let deadline = Instant::now() + Duration::from_secs(60);
    while w.c.t.exec("kopano.export", json!({})).is_err() {
        if Instant::now() > deadline {
            bail!("window {name} never showed the document");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(w)
}

fn shot(w: &mut Window, step: &str) -> Result<()> {
    w.c.t.call("ui.set", json!({"fit": true}))?;
    std::thread::sleep(Duration::from_millis(600));
    w.c.t.call("ui.screenshot", json!({"path": format!("{step}.png")}))?;
    Ok(())
}

fn repo<'a>(store: &'a MemStore, provider: &Provider, project: &Id, who: &str) -> Result<Repo<'a>> {
    let key = DeviceKey::generate();
    let cert = provider.issue(who, who, &key, "photo-proof", 24 * 3_600_000)?;
    Ok(Repo { store, project: Project::load(store, project)?, head: None, cert, key })
}

/// Items that differ between two snapshots.
fn changed(before: &BTreeMap<String, Item>, after: &BTreeMap<String, Item>) -> Vec<String> {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter().filter(|k| before.get(*k) != after.get(*k)).cloned().collect()
}

/// Move the media a version names that the other side lacks (what the node's fetch would do).
fn move_media(items: &BTreeMap<String, Item>, from: &Path, to: &Path) -> Result<(u64, u64, u64)> {
    let (mut n, mut raw, mut zst) = (0, 0, 0);
    for a in items.values().flat_map(|i| &i.assets) {
        let dst = to.join(&a.hash);
        if dst.exists() {
            continue;
        }
        let bytes = std::fs::read(from.join(&a.hash))?;
        zst += ruzstd::encoding::compress_to_vec(bytes.as_slice(), ruzstd::encoding::CompressionLevel::Fastest).len() as u64;
        raw += bytes.len() as u64;
        n += 1;
        std::fs::create_dir_all(to)?;
        std::fs::write(dst, bytes)?;
    }
    Ok((n, raw, zst))
}

struct Lab<'a> {
    a: Window,
    b: Window,
    ra: Repo<'a>,
    rb: Repo<'a>,
    sa: &'a MemStore,
    sb: &'a MemStore,
    snap_a: BTreeMap<String, Item>,
    log: Vec<Value>,
}

impl Lab<'_> {
    /// Alice saves her edit, it travels to Bob, Bob brings it in. Returns the measurement row.
    fn send(&mut self, step: &str, what: &str) -> Result<()> {
        let t0 = Instant::now();
        let snap = self.a.c.snapshot()?;
        let keys = changed(&self.snap_a, &snap);
        let v = self.ra.save(&mut self.a.c, what)?.context("the edit saved nothing")?;
        let save_ms = t0.elapsed().as_millis();
        let (objects, bytes) = copy_version_closure(self.sa, self.sb, &v)?;
        let delta: BTreeMap<String, Item> = keys.iter().filter_map(|k| snap.get(k).map(|i| (k.clone(), i.clone()))).collect();
        let (media_n, media_raw, media_zst) = move_media(&delta, &self.a.media, &self.b.media)?;
        let t1 = Instant::now();
        let staged_before = self.b.c.traffic.staged_zstd_bytes;
        let r = self.rb.integrate(&mut self.b.c, &v)?;
        self.b.c.flush()?;
        let apply_ms = t1.elapsed().as_millis();
        let applied = match r {
            Integrated::FastForward { applied } => applied,
            Integrated::Merged { applied, .. } => applied,
            Integrated::AlreadyHad => 0,
        };
        let tiles_changed = keys.iter().filter(|k| k.contains('@')).count();
        let tiles_total = snap.keys().filter(|k| k.contains('@')).count();
        // Both windows must now hold the same items.
        let b_snap = self.b.c.snapshot()?;
        let same = b_snap == snap;
        let row = json!({
            "step": step, "edit": what,
            "items_changed": keys.len(), "tile_items_changed": tiles_changed, "tile_items_in_document": tiles_total,
            "kopano_objects": objects, "kopano_bytes": bytes,
            "media_blobs": media_n, "media_raw_bytes": media_raw, "media_zstd_bytes": media_zst,
            "photo_staged_zstd_bytes_at_b": self.b.c.traffic.staged_zstd_bytes - staged_before,
            "applied_items_at_b": applied, "save_ms": save_ms, "bring_in_ms": apply_ms,
            "windows_match": same, "changed_keys": keys.iter().take(12).collect::<Vec<_>>(),
        });
        println!("{row}");
        if !same {
            bail!("after {step} the two windows differ");
        }
        self.snap_a = snap;
        self.log.push(row);
        shot(&mut self.a, step)?;
        shot(&mut self.b, step)?;
        Ok(())
    }
}

fn uid_of(snap: &BTreeMap<String, Item>, name: &str) -> Option<String> {
    snap.iter().find(|(k, i)| !k.contains('@') && i.state.get("name").and_then(Value::as_str) == Some(name)).and_then(|(k, _)| k.strip_prefix("layer/")).map(String::from)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let exe = arg(&args, "--exe").context("--exe <photocraft.exe>")?;
    let doc = PathBuf::from(arg(&args, "--doc").context("--doc <file.pcraft>")?);
    let out = PathBuf::from(arg(&args, "--out").context("--out <dir>")?);
    let ports: Vec<u16> = arg(&args, "--ports").unwrap_or_else(|| "18301,18302".into()).split(',').filter_map(|p| p.parse().ok()).collect();
    std::fs::create_dir_all(&out)?;

    let provider = Provider::generate();
    let (sa, sb) = (MemStore::default(), MemStore::default());
    let owner = DeviceKey::generate();
    let cert = provider.issue("alice", "Alice", &owner, "photo-proof", 24 * 3_600_000)?;
    let mut members = BTreeMap::new();
    members.insert("bob".to_string(), Member { role: Role::Editor, lanes: vec![] });
    let p = Project::create(&sa, "Owner photo", vec![provider.public_hex()], &cert, &owner, members)?;
    sb.put_verified(&p.id, &sa.get(&p.id)?)?;

    let a = launch(&exe, &out, "a", ports[0], &doc)?;
    let b = launch(&exe, &out, "b", ports[1], &doc)?;
    let mut lab = Lab { a, b, ra: repo(&sa, &provider, &p.id, "alice")?, rb: repo(&sb, &provider, &p.id, "bob")?, sa: &sa, sb: &sb, snap_a: BTreeMap::new(), log: vec![] };

    // 0. Both open the same photo; Bob starts from Alice's first version.
    let t0 = Instant::now();
    lab.snap_a = lab.a.c.snapshot()?;
    let v0 = lab.ra.save(&mut lab.a.c, "start")?.context("nothing saved")?;
    let first_ms = t0.elapsed().as_millis();
    let (objects, bytes) = copy_version_closure(&sa, &sb, &v0)?;
    // Bob saves his own copy first, so this is a merge of two identical starts: nothing to apply.
    let bob_applied = match lab.rb.integrate(&mut lab.b.c, &v0)? {
        Integrated::FastForward { applied } | Integrated::Merged { applied, .. } => applied,
        Integrated::AlreadyHad => 0,
    };
    let tiles = lab.snap_a.keys().filter(|k| k.contains('@')).count();
    let media: u64 = lab.snap_a.values().flat_map(|i| &i.assets).map(|a| a.size).sum();
    let row = json!({"step": "0-start", "edit": "both open the same photo", "items": lab.snap_a.len(), "tile_items": tiles, "media_raw_bytes_named": media,
        "kopano_objects": objects, "kopano_bytes": bytes, "first_snapshot_and_save_ms": first_ms, "bob_items_applied": bob_applied,
        "photo_fetched_zstd_bytes_at_a": lab.a.c.traffic.fetched_zstd_bytes});
    println!("{row}");
    lab.log.push(row);
    shot(&mut lab.a, "0-start")?;
    shot(&mut lab.b, "0-start")?;

    // 1. Adjustment: a new Exposure layer, then a change to its slider only.
    lab.a.c.t.exec("layer.newAdjustmentLayer.exposure", json!({"exposure": 0.3, "offset": 0.0, "gamma": 1.0}))?;
    lab.send("1a-new-exposure-layer", "Alice adds an Exposure adjustment layer (+0.3)")?;
    lab.a.c.t.exec("layer.setAdjustment", json!({"exposure": 0.7}))?;
    lab.send("1b-exposure-change", "Alice moves the Exposure slider to +0.7")?;

    // 2. Retouch: a new empty layer, then one brush stroke on it.
    lab.a.c.t.exec("layer.new.layer", json!({"name": "Retouch"}))?;
    lab.send("2a-new-retouch-layer", "Alice adds an empty Retouch layer")?;
    lab.a.c.t.exec("paint.stroke", json!({"points": [[400, 1880, 1], [800, 1930, 1], [1200, 1890, 1]], "size": 90, "color": "#ffd23f"}))?;
    lab.send("2b-one-brush-stroke", "Alice paints one brush stroke on Retouch")?;

    // 3. Both paint at once on the same layer, on different tiles; then each brings in the other.
    let uid = uid_of(&lab.snap_a, "Retouch").context("no Retouch layer")?;
    let id_b = lab.b.c.layer_id(&uid)?;
    lab.b.c.t.exec("layer.select", json!({"layer": id_b}))?;
    lab.a.c.t.exec("paint.stroke", json!({"points": [[150, 300, 1], [500, 380, 1]], "size": 70, "color": "#3fa7ff"}))?;
    lab.b.c.t.exec("paint.stroke", json!({"points": [[2400, 250, 1], [2800, 330, 1]], "size": 70, "color": "#ff3f9a"}))?;
    shot(&mut lab.a, "3a-both-painted-before-sync")?;
    shot(&mut lab.b, "3a-both-painted-before-sync")?;
    lab.ra.save(&mut lab.a.c, "Alice: blue stroke top left")?.context("alice saved nothing")?;
    let vb = lab.rb.save(&mut lab.b.c, "Bob: pink stroke top right")?.context("bob saved nothing")?;
    let snap_b = lab.b.c.snapshot()?;
    let snap_a = lab.a.c.snapshot()?;
    copy_version_closure(&sb, &sa, &vb)?;
    move_media(&snap_b, &lab.b.media, &lab.a.media)?;
    let merged = match lab.ra.integrate(&mut lab.a.c, &vb)? {
        Integrated::Merged { version, conflicts, applied } => {
            println!("{}", json!({"step": "3b-merge", "applied_at_a": applied, "clashes": conflicts.len()}));
            if !conflicts.is_empty() {
                bail!("strokes on different tiles clashed: {:?}", conflicts.iter().map(|c| &c.key).collect::<Vec<_>>());
            }
            version
        }
        _ => bail!("expected a merge"),
    };
    lab.a.c.flush()?;
    copy_version_closure(&sa, &sb, &merged)?;
    move_media(&snap_a, &lab.a.media, &lab.b.media)?;
    let r = lab.rb.integrate(&mut lab.b.c, &merged)?;
    lab.b.c.flush()?;
    let (fa, fb) = (lab.a.c.snapshot()?, lab.b.c.snapshot()?);
    let row = json!({"step": "3b-merged", "edit": "each brings in the other's stroke", "bob": format!("{:?}", matches!(r, Integrated::FastForward { .. })), "windows_match": fa == fb, "clashes": 0,
        "alice_tiles_changed": changed(&lab.snap_a, &snap_a).iter().filter(|k| k.contains('@')).count(),
        "bob_tiles_changed": changed(&lab.snap_a, &snap_b).iter().filter(|k| k.contains('@')).count()});
    println!("{row}");
    lab.log.push(row);
    shot(&mut lab.a, "3b-merged")?;
    shot(&mut lab.b, "3b-merged")?;
    if fa != fb {
        bail!("after the merge the windows differ");
    }

    std::fs::write(out.join("measurements.json"), serde_json::to_string_pretty(&lab.log)?)?;
    println!("wrote {}", out.join("measurements.json").display());
    Ok(())
}
