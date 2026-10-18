//! Security pass (2026-10-18): targeted attacks on the v2 workflow rules. Each test is an attack
//! that worked before the fix it names; it must now fail.
//! Run: cargo test --release -p kopano-lab --test attacks

use anyhow::Result;
use kopano_core::identity::{now_ms, DeviceCert, DeviceKey, Provider, Signed};
use kopano_core::project::{self, Member, Project, Role};
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::workflow::{self as wf, MemberEntry, MemberOp, RefUpdate, SignedMemberEntry, SignedRef, MAIN};
use std::collections::{BTreeMap, BTreeSet};

struct Who {
    key: DeviceKey,
    cert: DeviceCert,
}

struct Lab {
    store: MemStore,
    provider: Provider,
    project: Project,
    owner: Who,
    bob: Who,
    v0: Id,
}

fn who(p: &Provider, name: &str) -> Who {
    let key = DeviceKey::generate();
    let cert = p.issue(&format!("acct-{name}"), name, &key, "device", 86_400_000).unwrap();
    Who { key, cert }
}

fn lab(bob_role: Role) -> Lab {
    let store = MemStore::default();
    let provider = Provider::generate();
    let (owner, bob) = (who(&provider, "owner"), who(&provider, "bob"));
    let mut members = BTreeMap::new();
    members.insert(bob.cert.body.account.clone(), Member { role: bob_role, lanes: vec![] });
    let project = Project::create(&store, "attacks", vec![provider.public_hex()], &owner.cert, &owner.key, members).unwrap();
    let v0 = project::commit(&store, &project, vec![], None, "First upload", &owner.cert, &owner.key).unwrap();
    Lab { store, provider, project, owner, bob, v0 }
}

fn entry(l: &Lab, w: &Who, after: &[Id], op: MemberOp) -> Id {
    wf::add_member_entry(&l.store, &l.project, after.to_vec(), op, &w.cert, &w.key).unwrap()
}

/// A ref update with every field chosen by the attacker.
fn raw_ref(l: &Lab, w: &Who, name: &str, version: &Id, previous: Option<Id>, seq: u64, members: &[Id]) -> Id {
    let u = RefUpdate { kind: "kopano.ref".into(), project: l.project.id.hex(), name: name.into(), version: version.hex(), previous: previous.map(|p| p.hex()), seq, members: members.iter().map(Id::hex).collect(), time_ms: now_ms() };
    let s = SignedRef { update: Signed::new(u, w.key.signing()).unwrap(), cert: w.cert.clone() };
    l.store.put(&serde_json::to_vec(&s).unwrap()).unwrap()
}

fn main_tip(l: &Lab, members: &BTreeSet<Id>, refs: &[Id]) -> Result<Id> {
    let m = wf::members(&l.store, &l.project, members)?;
    let (t, _) = wf::tips(&l.store, &l.project, &m, &refs.iter().copied().collect());
    Ok(t[MAIN].0)
}

/// A1: Bob is made an approver, then demoted. He approves into main anyway, citing only the members
/// from before his demotion. Before the fix his update became main for everyone.
#[test]
fn a1_demoted_approver_cannot_take_main() -> Result<()> {
    let l = lab(Role::Editor);
    let e1 = entry(&l, &l.owner, &[], MemberOp::Set { account: l.bob.cert.body.account.clone(), role: Role::Editor, lanes: vec![], approver: true });
    let t = wf::update_ref(&l.store, &l.project, MAIN, &l.v0, None, &[e1], &l.owner.cert, &l.owner.key)?;
    let d = entry(&l, &l.owner, &[e1], MemberOp::Set { account: l.bob.cert.body.account.clone(), role: Role::Editor, lanes: vec![], approver: false });
    let seal = {
        let m = wf::members(&l.store, &l.project, &[e1].into())?;
        let (tips, _) = wf::tips(&l.store, &l.project, &m, &[t].into());
        wf::seal_main(&l.store, &l.project, (tips[MAIN].0, &tips[MAIN].1), &[d], &l.owner.cert, &l.owner.key)?
    };
    let evil = project::commit(&l.store, &l.project, vec![l.v0], None, "bob's own approval", &l.bob.cert, &l.bob.key)?;
    let u = wf::update_ref(&l.store, &l.project, MAIN, &evil, Some((t, 1)), &[e1], &l.bob.cert, &l.bob.key)?;
    let all: BTreeSet<Id> = [e1, d].into();
    assert_eq!(main_tip(&l, &all, &[t, seal, u])?, seal, "a demoted approver's backdated approval must lose to the owner's seal");
    // and he cannot build on the seal while citing the old members
    let u2 = wf::update_ref(&l.store, &l.project, MAIN, &evil, Some((seal, 2)), &[e1], &l.bob.cert, &l.bob.key)?;
    assert_eq!(main_tip(&l, &all, &[t, seal, u, u2])?, seal, "building on main must cite at least what main cited");
    Ok(())
}

/// A2: an approver claims a huge seq to win every fork. Before the fix it won.
#[test]
fn a2_seq_must_follow_previous() -> Result<()> {
    let l = lab(Role::Admin);
    let t = wf::update_ref(&l.store, &l.project, MAIN, &l.v0, None, &[], &l.owner.cert, &l.owner.key)?;
    let v1 = project::commit(&l.store, &l.project, vec![l.v0], None, "owner", &l.owner.cert, &l.owner.key)?;
    let honest = wf::update_ref(&l.store, &l.project, MAIN, &v1, Some((t, 1)), &[], &l.owner.cert, &l.owner.key)?;
    let v2 = project::commit(&l.store, &l.project, vec![l.v0], None, "bob", &l.bob.cert, &l.bob.key)?;
    let greedy = raw_ref(&l, &l.bob, MAIN, &v2, Some(t), 1_000_000_000, &[]);
    assert_eq!(main_tip(&l, &BTreeSet::new(), &[t, honest, greedy])?, honest);
    Ok(())
}

/// A3: Bob, an admin, is demoted. He signs an entry that cites nothing newer than his admin days,
/// grinds its id to sort before the demotion, and adds a sock-puppet admin. Before the fix the
/// puppet stayed an admin.
#[test]
fn a3_demoted_admin_cannot_plant_a_puppet() -> Result<()> {
    let l = lab(Role::Admin);
    let d = entry(&l, &l.owner, &[], MemberOp::Remove { account: l.bob.cert.body.account.clone() });
    let eve = who(&l.provider, "eve");
    let mut tries = 0;
    let puppet = loop {
        tries += 1;
        let e = MemberEntry { kind: "kopano.members".into(), project: l.project.id.hex(), after: vec![], op: MemberOp::Set { account: eve.cert.body.account.clone(), role: Role::Admin, lanes: vec![], approver: true }, time_ms: now_ms() - tries };
        let s = SignedMemberEntry { entry: Signed::new(e, l.bob.key.signing())?, cert: l.bob.cert.clone() };
        let id = l.store.put(&serde_json::to_vec(&s)?)?;
        if id < d {
            break id;
        }
    };
    let m = wf::members(&l.store, &l.project, &[d, puppet].into())?;
    assert!(!m.is_member(&l.bob.cert.body.account));
    assert!(!m.is_admin(&eve.cert.body.account), "puppet admin planted after {tries} tries");
    // an entry Bob made BEFORE the removal, which the owner saw, still stands
    let carol = who(&l.provider, "carol");
    let fair = entry(&l, &l.bob, &[], MemberOp::Set { account: carol.cert.body.account.clone(), role: Role::Editor, lanes: vec![], approver: false });
    let d2 = entry(&l, &l.owner, &[fair], MemberOp::Remove { account: l.bob.cert.body.account.clone() });
    let m = wf::members(&l.store, &l.project, &[fair, d2].into())?;
    assert!(m.is_member(&carol.cert.body.account), "work the remover had seen must stay");
    Ok(())
}

/// A4: a removed member keeps pushing their branch; a ref from another project is replayed here.
#[test]
fn a4_removed_member_and_foreign_refs() -> Result<()> {
    let l = lab(Role::Editor);
    let d = entry(&l, &l.owner, &[], MemberOp::Remove { account: l.bob.cert.body.account.clone() });
    let m = wf::members(&l.store, &l.project, &[d].into())?;
    let b = wf::update_ref(&l.store, &l.project, &wf::branch_of(&l.bob.cert.body.account), &l.v0, None, &[], &l.bob.cert, &l.bob.key)?;
    assert!(wf::check_ref(&l.store, &l.project, &m, &b).is_err());
    let other = lab(Role::Editor);
    let foreign = wf::update_ref(&other.store, &other.project, MAIN, &other.v0, None, &[], &other.owner.cert, &other.owner.key)?;
    l.store.put_verified(&foreign, &other.store.get(&foreign)?)?;
    assert!(wf::check_ref(&l.store, &l.project, &m, &foreign).is_err());
    // the other project's owner is not our owner either
    let fake_main = wf::update_ref(&l.store, &l.project, MAIN, &l.v0, None, &[], &other.owner.cert, &other.owner.key)?;
    assert!(wf::check_ref(&l.store, &l.project, &m, &fake_main).is_err());
    Ok(())
}
