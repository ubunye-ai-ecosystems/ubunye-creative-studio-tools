//! Security pass (2026-10-18): everything another device can send us, mutated at random. Nothing
//! may panic, nothing may allocate without bound, and no mutated signed object may be accepted
//! as something other than what was signed.
//! Run: FUZZ_ITERS=1000000 cargo test --release -p kopano-lab --test fuzz_inputs -- --nocapture

use kopano_core::identity::{DeviceKey, Provider};
use kopano_core::project::{self, Member, Project, Role};
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::tree;
use kopano_core::wire;
use kopano_core::workflow::{self as wf, MemberOp, MAIN};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn iters() -> u64 {
    std::env::var("FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000)
}

fn mutate(rng: &mut StdRng, input: &[u8]) -> Vec<u8> {
    let mut b = input.to_vec();
    for _ in 0..rng.gen_range(1..=4) {
        match rng.gen_range(0..6) {
            0 if !b.is_empty() => {
                let i = rng.gen_range(0..b.len());
                b[i] ^= 1 << rng.gen_range(0..8);
            }
            1 if !b.is_empty() => {
                let i = rng.gen_range(0..b.len());
                b[i] = rng.gen();
            }
            2 => b.truncate(rng.gen_range(0..=b.len())),
            3 => {
                let i = rng.gen_range(0..=b.len());
                let n = rng.gen_range(1..16);
                for _ in 0..n {
                    b.insert(i, rng.gen());
                }
            }
            4 if b.len() >= 4 => {
                // a length field set to something huge
                let i = rng.gen_range(0..b.len() - 3);
                b[i..i + 4].copy_from_slice(&[0xff, 0xff, 0xff, rng.gen_range(0..0x80)]);
            }
            _ => {
                let i = rng.gen_range(0..=b.len());
                let j = rng.gen_range(i..=b.len());
                let chunk = b[i..j].to_vec();
                b.splice(i..i, chunk);
            }
        }
    }
    b
}

fn record(out: &mut Vec<u8>, kind: u8, data: &[u8]) {
    out.push(kind);
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
}

/// A real store with a tree, an edited tree and a version; and a valid compact payload between them.
struct Fixture {
    store: MemStore,
    payload: Vec<u8>,
    old_node: Vec<u8>,
    delta: Vec<u8>,
    signed: Vec<Vec<u8>>,
    project: Project,
    known_refs: Vec<Id>,
    known_members: Vec<Id>,
}

fn fixture() -> Fixture {
    let store = MemStore::default();
    let provider = Provider::generate();
    let (ok, bk) = (DeviceKey::generate(), DeviceKey::generate());
    let oc = provider.issue("acct-owner", "owner", &ok, "d", 86_400_000).unwrap();
    let bc = provider.issue("acct-bob", "bob", &bk, "d", 86_400_000).unwrap();
    let mut members = BTreeMap::new();
    members.insert("acct-bob".to_string(), Member { role: Role::Editor, lanes: vec![] });
    let project = Project::create(&store, "fuzz", vec![provider.public_hex()], &oc, &ok, members).unwrap();
    let item = |x: i64| project::Item { kind: "lab.develop".into(), lane: "develop".into(), schema: 1, state: serde_json::json!({"exposure": x}), assets: vec![] }.put(&store).unwrap();
    let r1 = tree::update(&store, None, (0..40).map(|i| (format!("IMG_{i:04}#develop"), Some(item(i)))).collect()).unwrap();
    let r2 = tree::update(&store, r1, vec![("IMG_0003#develop".into(), Some(item(99)))]).unwrap();
    let old_node = store.get(&r1.unwrap()).unwrap();
    let new_node = store.get(&r2.unwrap()).unwrap();
    let delta = tree::node_delta(&old_node, &new_node).expect("delta");
    let v1 = project::commit(&store, &project, vec![], r1, "first", &oc, &ok).unwrap();
    let v2 = project::commit(&store, &project, vec![v1], r2, "edit", &oc, &ok).unwrap();
    let mut raw = vec![];
    let mut rec = r1.unwrap().0.to_vec();
    rec.extend(&delta);
    record(&mut raw, 1, &rec);
    record(&mut raw, 0, &store.get(&v2).unwrap());
    let payload = miniz_oxide::deflate::compress_to_vec(&raw, 6);
    let e1 = wf::add_member_entry(&store, &project, vec![], MemberOp::Set { account: "acct-bob".into(), role: Role::Editor, lanes: vec![], approver: true }, &oc, &ok).unwrap();
    let t = wf::update_ref(&store, &project, MAIN, &v2, None, &[e1], &oc, &ok).unwrap();
    let b = wf::update_ref(&store, &project, &wf::branch_of("acct-bob"), &v1, None, &[e1], &bc, &bk).unwrap();
    let signed = [v2, e1, t, b].iter().map(|id| store.get(id).unwrap()).collect();
    Fixture { store, payload, old_node, delta, signed, project, known_refs: vec![t, b], known_members: vec![e1] }
}

#[test]
fn fuzz_inputs() {
    let n = iters();
    let f = fixture();
    let raw_len = miniz_oxide::inflate::decompress_to_vec(&f.payload).unwrap().len() as u64;
    let mut rng = StdRng::seed_from_u64(20261018);
    let mut panics: BTreeMap<&str, u64> = BTreeMap::new();
    let mut accepted_forgeries = 0u64;
    let mut decoded_ok = 0u64;
    let original_members = wf::members(&f.store, &f.project, &f.known_members.iter().copied().collect()).unwrap();
    let original_tips: Vec<(String, String)> = wf::tips(&f.store, &f.project, &original_members, &f.known_refs.iter().copied().collect()).0.into_iter().map(|(k, (_, u))| (k, u.version)).collect();
    for i in 0..n {
        // 1. compressed payloads: mutated valid ones and pure noise, with honest or lying sizes
        let z = if i % 4 == 0 { (0..rng.gen_range(0..256)).map(|_| rng.gen()).collect() } else { mutate(&mut rng, &f.payload) };
        let declared = match rng.gen_range(0..4) {
            0 => raw_len,
            1 => rng.gen_range(0..4096),
            2 => wire::MAX_RAW,
            _ => u64::MAX,
        };
        let scratch = MemStore::default();
        for id in [Id::of(&f.old_node)] {
            scratch.put_verified(&id, &f.old_node).unwrap();
        }
        match catch_unwind(AssertUnwindSafe(|| wire::decode_compact(&scratch, &z, declared))) {
            Err(_) => *panics.entry("decode_compact").or_default() += 1,
            Ok(Ok(_)) => decoded_ok += 1,
            Ok(Err(_)) => {}
        }
        // 2. node deltas and tree nodes
        let d = mutate(&mut rng, &f.delta);
        if catch_unwind(AssertUnwindSafe(|| {
            let _ = tree::apply_node_delta(&f.old_node, &d);
            let _ = tree::children(&d);
            let _ = tree::children(&mutate(&mut rng.clone(), &f.old_node));
        }))
        .is_err()
        {
            *panics.entry("tree").or_default() += 1;
        }
        // 3. signed objects: a mutated copy may parse, but must never change what is accepted
        let pick = &f.signed[rng.gen_range(0..f.signed.len())];
        let m = mutate(&mut rng, pick);
        let r = catch_unwind(AssertUnwindSafe(|| {
            let s = MemStore::default();
            for id in f.known_refs.iter().chain(&f.known_members) {
                s.put_verified(id, &f.store.get(id).unwrap()).unwrap();
            }
            s.put_verified(&f.project.id, &f.store.get(&f.project.id).unwrap()).unwrap();
            let mid = Id::of(&m);
            s.put_verified(&mid, &m).unwrap();
            let _ = project::verify_version(&s, &f.project, &mid);
            let mut members: BTreeSet<Id> = f.known_members.iter().copied().collect();
            members.insert(mid);
            let cur = wf::members(&s, &f.project, &members).unwrap();
            let mut refs: BTreeSet<Id> = f.known_refs.iter().copied().collect();
            refs.insert(mid);
            let tips: Vec<(String, String)> = wf::tips(&s, &f.project, &cur, &refs).0.into_iter().map(|(k, (_, u))| (k, u.version)).collect();
            // the mutated object is a forgery unless it is byte-identical to a signed one
            let same_members = cur.map == original_members.map && cur.blocked == original_members.blocked;
            (tips == original_tips && same_members) || f.signed.contains(&m)
        }));
        match r {
            Err(_) => *panics.entry("signed objects").or_default() += 1,
            Ok(false) => accepted_forgeries += 1,
            Ok(true) => {}
        }
    }
    // 4. deflate bombs: 200 MB of zeros packed into a small frame
    let bomb = miniz_oxide::deflate::compress_to_vec(&vec![0u8; 200 << 20], 10);
    let s = MemStore::default();
    let refused_big = wire::decode_compact(&s, &bomb, 200 << 20).is_err();
    let refused_lie = wire::decode_compact(&s, &bomb, 1 << 20).is_err();
    println!(
        "{}",
        serde_json::json!({"iterations": n, "panics": panics, "accepted_forgeries": accepted_forgeries, "mutated_payloads_decoded": decoded_ok,
            "bomb_compressed_bytes": bomb.len(), "bomb_refused_when_declared": refused_big, "bomb_refused_when_size_lies": refused_lie})
    );
    assert!(panics.is_empty(), "panics: {panics:?}");
    assert_eq!(accepted_forgeries, 0);
    assert!(refused_big && refused_lie);
}
