//! Conformance kit: the checks every connector must pass before an app counts as supported.
//! The app supplies a `Harness`: a way to make fresh instances and to change one numeric setting,
//! the minimum needed to exercise the contract generically.

use crate::connector::Connector;
use crate::identity::{DeviceKey, Provider};
use crate::project::{Item, Project};
use crate::repo::{copy_version_closure, Integrated, Repo};
use crate::store::{Id, MemStore, ObjectStore};
use anyhow::{anyhow, bail, ensure, Result};
use std::collections::BTreeMap;

pub trait Harness: Connector {
    /// A fresh app instance holding the same starting items as every other fresh instance.
    fn fresh(&self) -> Self
    where
        Self: Sized;
    /// Change one numeric setting (a path in the item state) of one item, through the app.
    fn set_number(&mut self, key: &str, path: &str, value: f64) -> Result<()>;
    /// Read the same setting back from the app.
    fn get_number(&mut self, key: &str, path: &str) -> Result<f64>;
    /// A key and two different mergeable numeric settings on it, for the merge tests.
    fn sample(&self) -> (String, String, String);
}

pub struct Outcome {
    pub name: &'static str,
    pub result: Result<()>,
}

struct Env {
    provider: Provider,
    stores: [MemStore; 2],
}

fn repo<'a>(env: &'a Env, i: usize, project: &Project, who: &str) -> Result<Repo<'a>> {
    let key = DeviceKey::generate();
    let cert = env.provider.issue(who, who, &key, "test", 3_600_000)?;
    Ok(Repo { store: &env.stores[i], project: Project::load(&env.stores[i], &project.id)?, head: None, cert, key })
}

fn setup<H: Harness>(h: &H) -> Result<(Env, Project, H, H)> {
    let env = Env { provider: Provider::generate(), stores: [MemStore::default(), MemStore::default()] };
    let owner = DeviceKey::generate();
    let cert = env.provider.issue("alice", "Alice", &owner, "test", 3_600_000)?;
    let mut members = BTreeMap::new();
    members.insert("bob".to_string(), crate::project::Member { role: crate::project::Role::Editor, lanes: vec![] });
    let p = Project::create(&env.stores[0], "conformance", vec![env.provider.public_hex()], &cert, &owner, members)?;
    env.stores[1].put_verified(&p.id, &env.stores[0].get(&p.id)?)?;
    Ok((env, p, h.fresh(), h.fresh()))
}

pub fn run<H: Harness>(h: &H) -> Vec<Outcome> {
    vec![
        Outcome { name: "apply(snapshot) changes nothing", result: apply_is_identity(h) },
        Outcome { name: "save, change, restore gives the exact starting state", result: roundtrip(h) },
        Outcome { name: "different settings changed by two people both survive", result: merge_disjoint(h) },
        Outcome { name: "same setting changed by two people is a clash, both kept", result: merge_clash(h) },
        Outcome { name: "bringing in the same version twice changes nothing", result: idempotent(h) },
        Outcome { name: "a changed version is refused", result: tamper(h) },
        Outcome { name: "items carry media by hash only, with declared kinds", result: declared(h) },
    ]
}

fn apply_is_identity<H: Harness>(h: &H) -> Result<()> {
    let mut a = h.fresh();
    let snap = a.snapshot()?;
    for (k, item) in &snap {
        a.apply(k, Some(item))?;
    }
    ensure!(a.snapshot()? == snap, "applying the app's own items changed them");
    Ok(())
}

fn roundtrip<H: Harness>(h: &H) -> Result<()> {
    let (env, p, mut a, _) = setup(h)?;
    let mut r = repo(&env, 0, &p, "alice")?;
    let start = a.snapshot()?;
    let v0 = r.save(&mut a, "start")?.ok_or_else(|| anyhow!("first save produced nothing"))?;
    let (key, f1, _) = h.sample();
    a.set_number(&key, &f1, 0.7)?;
    r.save(&mut a, "edit")?;
    r.restore(&mut a, &v0)?;
    ensure!(a.snapshot()? == start, "restore did not return the exact starting state");
    ensure!(crate::project::history(r.store, &[r.head.unwrap_or(v0)])?.len() >= 3, "restore lost history");
    Ok(())
}

struct Branches<H> {
    env: Env,
    project: Project,
    alice_app: H,
    alice_head: Id,
    bob_head: Id,
    key: String,
    f1: String,
    f2: String,
}

/// Alice and Bob start from the same version, each changes one setting, Bob's work is copied to
/// Alice's store (as sync would). `same_field` chooses whether they touch the same setting.
fn two_branches<H: Harness>(h: &H, same_field: bool) -> Result<Branches<H>> {
    let (env, project, mut a, mut b) = setup(h)?;
    let (key, f1, f2) = h.sample();
    let (alice_head, bob_head) = {
        let mut ra = repo(&env, 0, &project, "alice")?;
        let v0 = ra.save(&mut a, "start")?.ok_or_else(|| anyhow!("nothing saved"))?;
        copy_version_closure(&env.stores[0], &env.stores[1], &v0)?;
        let mut rb = repo(&env, 1, &project, "bob")?;
        rb.integrate(&mut b, &v0)?;
        a.set_number(&key, &f1, 0.4)?;
        let va = ra.save(&mut a, "alice edit")?.ok_or_else(|| anyhow!("alice saved nothing"))?;
        b.set_number(&key, if same_field { &f1 } else { &f2 }, 0.9)?;
        let vb = rb.save(&mut b, "bob edit")?.ok_or_else(|| anyhow!("bob saved nothing"))?;
        copy_version_closure(&env.stores[1], &env.stores[0], &vb)?;
        (va, vb)
    };
    Ok(Branches { env, project, alice_app: a, alice_head, bob_head, key, f1, f2 })
}

fn merge_disjoint<H: Harness>(h: &H) -> Result<()> {
    let mut t = two_branches(h, false)?;
    let mut ra = repo(&t.env, 0, &t.project, "alice")?;
    ra.head = Some(t.alice_head);
    match ra.integrate(&mut t.alice_app, &t.bob_head)? {
        Integrated::Merged { conflicts, .. } => ensure!(conflicts.is_empty(), "unexpected clash: {conflicts:?}"),
        _ => bail!("expected a merge"),
    }
    ensure!((t.alice_app.get_number(&t.key, &t.f1)? - 0.4).abs() < 1e-9, "alice's change was lost");
    ensure!((t.alice_app.get_number(&t.key, &t.f2)? - 0.9).abs() < 1e-9, "bob's change was lost");
    let head = ra.head.ok_or_else(|| anyhow!("no head after merge"))?;
    crate::project::verify_version(&t.env.stores[0], &ra.project, &head)?;
    Ok(())
}

fn merge_clash<H: Harness>(h: &H) -> Result<()> {
    let mut t = two_branches(h, true)?;
    let mut ra = repo(&t.env, 0, &t.project, "alice")?;
    ra.head = Some(t.alice_head);
    match ra.integrate(&mut t.alice_app, &t.bob_head)? {
        Integrated::Merged { conflicts, .. } => ensure!(conflicts.len() == 1, "expected one clash, got {}", conflicts.len()),
        _ => bail!("expected a merge"),
    }
    // Since E17 a clash takes the same value whoever merges (deterministic pick), so the app shows
    // one of the two values and the record keeps both.
    let now = t.alice_app.get_number(&t.key, &t.f1)?;
    ensure!((now - 0.4).abs() < 1e-9 || (now - 0.9).abs() < 1e-9, "the merged value is neither person's");
    let recs = ra.conflicts()?;
    ensure!(recs.len() == 1, "the clash was not recorded");
    let rec = recs.values().next().ok_or_else(|| anyhow!("no record"))?;
    let ptr = format!("/{}", t.f1.replace('.', "/"));
    let mut kept = vec![];
    for side in ["mine", "theirs"] {
        let id = rec.state.get(side).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("record lacks {side}"))?;
        kept.extend(Item::get(&t.env.stores[0], &Id::parse(id)?)?.state.pointer(&ptr).and_then(|v| v.as_f64()));
    }
    kept.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ensure!(kept == vec![0.4, 0.9], "the record does not keep both values: {kept:?}");
    let head = ra.head.ok_or_else(|| anyhow!("no head after merge"))?;
    crate::project::verify_version(&t.env.stores[0], &ra.project, &head)?;
    Ok(())
}

fn idempotent<H: Harness>(h: &H) -> Result<()> {
    let (env, p, mut a, _) = setup(h)?;
    let mut r = repo(&env, 0, &p, "alice")?;
    let v = r.save(&mut a, "start")?.ok_or_else(|| anyhow!("nothing saved"))?;
    ensure!(matches!(r.integrate(&mut a, &v)?, Integrated::AlreadyHad), "integrating its own head did something");
    ensure!(matches!(r.integrate(&mut a, &v)?, Integrated::AlreadyHad), "second integrate did something");
    Ok(())
}

fn tamper<H: Harness>(h: &H) -> Result<()> {
    let (env, p, mut a, _) = setup(h)?;
    let mut r = repo(&env, 0, &p, "alice")?;
    let v = r.save(&mut a, "start")?.ok_or_else(|| anyhow!("nothing saved"))?;
    let bytes = env.stores[0].get(&v)?;
    let forged = String::from_utf8(bytes)?.replace("\"start\"", "\"forged\"");
    let fid = env.stores[1].put(forged.as_bytes())?;
    env.stores[1].put_verified(&p.id, &env.stores[0].get(&p.id)?)?;
    copy_version_closure(&env.stores[0], &env.stores[1], &v)?;
    let p1 = Project::load(&env.stores[1], &p.id)?;
    ensure!(crate::project::verify_version(&env.stores[1], &p1, &fid).is_err(), "a forged version was accepted");
    ensure!(crate::project::verify_version(&env.stores[1], &p1, &v).is_ok(), "the genuine version was refused");
    Ok(())
}

fn declared<H: Harness>(h: &H) -> Result<()> {
    let mut a = h.fresh();
    let info = a.info();
    ensure!(info.contract == crate::connector::CONTRACT_VERSION, "connector targets contract {}", info.contract);
    for (k, item) in a.snapshot()? {
        ensure!(info.owns(&item.kind), "item {k} has undeclared kind {}", item.kind);
        for asset in &item.assets {
            ensure!(asset.hash.len() == 64 && asset.hash.bytes().all(|b| b.is_ascii_hexdigit()), "item {k} references media by something other than a BLAKE3 hash");
        }
        ensure!(serde_json::to_vec(&item.state)?.len() < 1 << 20, "item {k} state is over 1 MiB: media embedded in state?");
    }
    Ok(())
}
