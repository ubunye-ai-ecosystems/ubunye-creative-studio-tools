//! Real-app proof for Ubunye Film: two Film windows, each with its own Kopano store and device
//! identity. The editor (window A) trims a clip and grades it; at the same time the sound person
//! (window B) changes the level of an audio clip. Each side saves a Kopano version through the
//! Film connector, the versions are exchanged (as sync would carry them), and each side brings
//! the other's in. Everything goes through Film's control channel; no OS-level input.
//!
//! Usage: film_proof <portA> <portB> <project.fcproj> <out dir>
//!        film_proof make-project <filmcraft-cli> <out.fcproj> <clip>...   (headless, no window)
//! Writes screenshots (before/after for both windows) and `film_proof.json` into <out dir>.

use anyhow::{anyhow, Context, Result};
use kopano_core::connector::Connector;
use kopano_core::identity::{DeviceKey, Provider};
use kopano_core::project::{Member, Project, Role};
use kopano_core::repo::{copy_version_closure, Integrated, Repo};
use kopano_core::store::{MemStore, ObjectStore};
use kopano_lab::film::{FilmConnector, Fresh};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const TPS: f64 = 254_016_000_000.0;

fn connect(port: u16, project: &str) -> Result<FilmConnector> {
    let t0 = Instant::now();
    loop {
        match FilmConnector::live(port, Fresh::Windows { ports: vec![port], project: project.into() }) {
            Ok(mut c) => {
                c.open(project)?;
                return Ok(c);
            }
            Err(e) if t0.elapsed() < Duration::from_secs(120) => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
}

fn shot(c: &mut FilmConnector, path: &str) -> Result<()> {
    // let decoders catch up on the frame under the playhead before the picture is taken
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(1500));
        c.t.call("ui.screenshot", json!({}))?;
    }
    c.t.call("ui.screenshot", json!({"path": path}))?;
    Ok(())
}

fn first_clip(c: &mut FilmConnector, kind: &str) -> Result<(u64, u64, Value)> {
    let all = c.t.exec("kopano.items", json!({}))?;
    let seq = all.pointer("/sequences/0").cloned().ok_or_else(|| anyhow!("no sequence"))?;
    let sid = seq["id"].as_u64().ok_or_else(|| anyhow!("no sequence id"))?;
    let clip = seq["clips"].as_array().and_then(|cs| cs.iter().find(|c| c["kind"] == json!(kind)).cloned()).ok_or_else(|| anyhow!("no {kind} clip"))?;
    Ok((sid, clip["id"].as_u64().unwrap_or(0), clip))
}

fn said(i: &Integrated) -> String {
    match i {
        Integrated::AlreadyHad => "already had it".into(),
        Integrated::FastForward { applied } => format!("fast-forward, {applied} items applied"),
        Integrated::Merged { applied, conflicts, .. } => format!("merged, {applied} items applied, {} clashes", conflicts.len()),
    }
}

/// One sequence of the given clips end to end, saved as a project file.
fn make_project(cli: &str, out: &str, clips: &[String]) -> Result<()> {
    let mut t = kopano_lab::film::Transport::headless(cli, None)?;
    t.exec("file.newProject", json!({"name": "Cable way"}))?;
    let r = t.exec("file.import", json!({"paths": clips}))?;
    let all = t.exec("kopano.items", json!({}))?;
    let mut items: Vec<(String, u64)> = all["media"].as_array().cloned().unwrap_or_default().iter().filter_map(|m| Some((m["path"].as_str()?.to_string(), m["item"].as_u64()?))).collect();
    // in the order given
    items.sort_by_key(|(p, _)| clips.iter().position(|c| std::path::Path::new(c) == std::path::Path::new(p)).unwrap_or(usize::MAX));
    let ids: Vec<u64> = items.iter().map(|x| x.1).collect();
    anyhow::ensure!(ids.len() == clips.len(), "imported {} of {} clips: {r}", ids.len(), clips.len());
    let seq = t.exec("file.newSequenceFromClip", json!({"items": ids}))?;
    // a neutral Lumetri on the first video clip: the colourist's starting point
    let all = t.exec("kopano.items", json!({}))?;
    let first = all.pointer("/sequences/0/clips").and_then(Value::as_array).and_then(|cs| cs.iter().find(|c| c["kind"] == json!("video")).cloned()).ok_or_else(|| anyhow!("no video clip"))?;
    t.exec("effects.apply", json!({"clips": [first["id"]], "effect": "lumetri"}))?;
    t.exec("file.saveAs", json!({"path": out}))?;
    println!("{}", json!({"project": out, "sequence": seq}));
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("make-project") && a.len() > 4 {
        return make_project(&a[2], &a[3], &a[4..]);
    }
    let (pa, pb, project, out) = match &a[1..] {
        [pa, pb, p, o] => (pa.parse::<u16>()?, pb.parse::<u16>()?, p.clone(), o.clone()),
        _ => anyhow::bail!("usage: film_proof <portA> <portB> <project.fcproj> <out dir>"),
    };
    std::fs::create_dir_all(&out)?;
    let shot_path = |name: &str| std::path::Path::new(&out).join(name).to_string_lossy().into_owned();
    let mut report = serde_json::Map::new();

    let mut ed = connect(pa, &project).context("window A")?;
    let mut snd = connect(pb, &project).context("window B")?;
    let (_seq, vclip, v0) = first_clip(&mut ed, "video")?;
    let (_, aclip, _) = first_clip(&mut ed, "audio")?;
    let mid = |cut: &Value| (cut[0].as_f64().unwrap_or(0.0) + cut[1].as_f64().unwrap_or(0.0) / 2.0) / TPS;
    let at = mid(&v0["edit"]["cut"]);
    for c in [&mut ed, &mut snd] {
        c.t.exec("playhead.set", json!({"seconds": at}))?;
    }
    report.insert("idBlocks".into(), json!([ed.t.exec("kopano.idBlock", json!({}))?["block"], snd.t.exec("kopano.idBlock", json!({}))?["block"]]));
    shot(&mut ed, &shot_path("1-before-A.png"))?;
    shot(&mut snd, &shot_path("1-before-B.png"))?;

    // ---- one Kopano project, two members, two stores (one per computer) ----
    let provider = Provider::generate();
    let (ka, kb) = (DeviceKey::generate(), DeviceKey::generate());
    let ca = provider.issue("thabang", "Thabang (editor)", &ka, "film-a", 3_600_000)?;
    let cb = provider.issue("mpho", "Mpho (sound)", &kb, "film-b", 3_600_000)?;
    let (sa, sb) = (MemStore::default(), MemStore::default());
    let mut members = BTreeMap::new();
    members.insert("mpho".to_string(), Member { role: Role::Editor, lanes: vec![] });
    let p = Project::create(&sa, "Cable way edit", vec![provider.public_hex()], &ca, &ka, members)?;
    sb.put_verified(&p.id, &sa.get(&p.id)?)?;
    let mut ra = Repo { store: &sa, project: Project::load(&sa, &p.id)?, head: None, cert: ca, key: ka };
    let mut rb = Repo { store: &sb, project: Project::load(&sb, &p.id)?, head: None, cert: cb, key: kb };
    let base = ra.save(&mut ed, "start: three cable way clips")?.ok_or_else(|| anyhow!("nothing saved"))?;
    copy_version_closure(&sa, &sb, &base)?;
    let joined = rb.integrate(&mut snd, &base)?;
    report.insert("join".into(), json!(said(&joined)));
    report.insert("items".into(), json!(ed.snapshot()?.len()));

    let (snap_a0, snap_b0) = (ed.snapshot()?, snd.snapshot()?);
    // ---- the editor trims and grades (window A), the sound person sets a level (window B) ----
    ed.t.exec("timeline.trim", json!({"clip": vclip, "edge": "in", "deltaFrames": 15}))?;
    for (param, value) in [("exposure", 0.9), ("temperature", 45.0), ("saturation", 150.0), ("contrast", 25.0)] {
        ed.t.exec("effects.setParam", json!({"clip": vclip, "effect": "lumetri", "param": param, "value": value}))?;
    }
    snd.t.exec("clip.audioGain", json!({"clips": [aclip], "mode": "set", "db": -9.0}))?;
    let va = ra.save(&mut ed, "trim the first shot, warm grade")?.ok_or_else(|| anyhow!("editor saved nothing"))?;
    let vb = rb.save(&mut snd, "first clip's sound down 9 dB")?.ok_or_else(|| anyhow!("sound saved nothing"))?;
    for (c, before, tag) in [(&mut ed, &snap_a0, "changes_A"), (&mut snd, &snap_b0, "changes_B")] {
        let now = c.snapshot()?;
        let mut lines = vec![];
        for (k, item) in &now {
            if before.get(k) != Some(item) {
                lines.push(format!("{k}: {}", c.describe(before.get(k), item).join("; ")));
            }
        }
        report.insert(tag.into(), json!(lines));
    }
    shot(&mut ed, &shot_path("2-edited-A.png"))?;
    shot(&mut snd, &shot_path("2-edited-B.png"))?;

    // ---- exchange: B brings in A's version, A brings in B's ----
    copy_version_closure(&sa, &sb, &va)?;
    copy_version_closure(&sb, &sa, &vb)?;
    let ib = rb.integrate(&mut snd, &va)?;
    let ia = ra.integrate(&mut ed, &vb)?;
    let conflicts = |i: &Integrated| match i {
        Integrated::Merged { conflicts, .. } => conflicts.len(),
        _ => 0,
    };
    report.insert("B_brings_in_A".into(), json!(said(&ib)));
    report.insert("A_brings_in_B".into(), json!(said(&ia)));
    report.insert("clashes".into(), json!(conflicts(&ib) + conflicts(&ia)));
    for c in [&mut ed, &mut snd] {
        c.t.exec("playhead.set", json!({"seconds": at}))?;
    }
    shot(&mut ed, &shot_path("3-after-A.png"))?;
    shot(&mut snd, &shot_path("3-after-B.png"))?;

    // ---- the two windows now hold the same timeline ----
    let (xa, xb) = (ed.snapshot()?, snd.snapshot()?);
    report.insert("identical_items".into(), json!(xa == xb));
    let diff: Vec<&String> = xa.keys().filter(|k| xa.get(*k) != xb.get(*k)).collect();
    report.insert("differing_keys".into(), json!(diff));
    let (_, _, v1) = first_clip(&mut snd, "video")?;
    report.insert("B_clip_cut_before".into(), v0["edit"]["cut"].clone());
    report.insert("B_clip_cut_after".into(), v1["edit"]["cut"].clone());
    report.insert("B_grade_exposure".into(), v1.pointer("/grade/fx/lumetri/params/exposure/value/Float").cloned().unwrap_or(Value::Null));
    let a_audio = ed.t.exec("kopano.items", json!({}))?;
    let gain = a_audio.pointer("/sequences/0/clips").and_then(Value::as_array).and_then(|cs| cs.iter().find(|c| c["id"] == json!(aclip)).cloned());
    report.insert("A_audio_gain_db".into(), gain.and_then(|g| g.pointer("/sound/gain_db").cloned()).unwrap_or(Value::Null));

    // ---- a frame rendered from the originals on each side: byte for byte the same ----
    let mut frames = vec![];
    for (c, tag) in [(&mut ed, "A"), (&mut snd, "B")] {
        let path = shot_path(&format!("frame-{tag}.png"));
        let _ = std::fs::remove_file(&path);
        c.t.exec("file.exportFrame", json!({"path": path, "time": (at * TPS) as i64, "target": "program"}))?;
        let bytes = std::fs::read(&path).with_context(|| format!("no frame at {path}"))?;
        frames.push(blake3::hash(&bytes).to_hex().to_string());
    }
    report.insert("frame_blake3".into(), json!(frames));
    report.insert("frames_identical".into(), json!(frames[0] == frames[1]));
    // ---- and a two-second CPU export of the sequence (software encoder, from the originals) ----
    let mut movies = vec![];
    for (c, tag) in [(&mut ed, "A"), (&mut snd, "B")] {
        let path = shot_path(&format!("export-{tag}.mp4"));
        let _ = std::fs::remove_file(&path);
        let t0 = Instant::now();
        let r = c.t.exec(
            "file.exportMedia",
            json!({"path": path, "format": "h264", "hardwareEncoding": "off", "range": "custom", "startSeconds": 0.0, "endSeconds": 2.0, "wait": true}),
        )?;
        let bytes = std::fs::read(&path).with_context(|| format!("no export at {path}: {r}"))?;
        movies.push(json!({"blake3": blake3::hash(&bytes).to_hex().to_string(), "bytes": bytes.len(), "seconds": t0.elapsed().as_secs_f64()}));
    }
    report.insert("export_identical".into(), json!(movies[0]["blake3"] == movies[1]["blake3"]));
    report.insert("exports".into(), json!(movies));
    std::fs::write(shot_path("film_proof.json"), serde_json::to_string_pretty(&Value::Object(report.clone()))?)?;
    println!("{}", serde_json::to_string_pretty(&Value::Object(report))?);
    Ok(())
}
