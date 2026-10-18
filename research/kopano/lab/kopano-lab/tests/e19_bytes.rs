//! E19: where the bytes of one small edit go, and how far each idea cuts them.
//! Run: cargo test --release -p kopano-lab --test e19_bytes -- --nocapture

use anyhow::Result;
use kopano_core::identity::{DeviceKey, Provider};
use kopano_core::project::{self, Project};
use kopano_core::repo::Repo;
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::tree;
use kopano_lab::ToyEditor;
use serde_json::json;
use std::collections::BTreeMap;

fn deflate(b: &[u8]) -> usize {
    miniz_oxide::deflate::compress_to_vec(b, 9).len()
}

fn kind(b: &[u8]) -> &'static str {
    match b.first() {
        Some(b'L') => "tree leaf",
        Some(b'B') => "tree branch",
        Some(b'{') if b.windows(9).any(|w| w == b"\"parents\"") => "version",
        Some(b'{') => "item",
        _ => "other",
    }
}

#[test]
fn e19_bytes() -> Result<()> {
    for n in [1_000usize, 10_000, 100_000] {
        let store = MemStore::default();
        let provider = Provider::generate();
        let key = DeviceKey::generate();
        let cert = provider.issue("acct-a", "A", &key, "pc", 86_400_000)?;
        let p = Project::create(&store, "bytes", vec![provider.public_hex()], &cert, &key, BTreeMap::new())?;
        let mut app = ToyEditor::demo(n);
        let mut repo = Repo { store: &store, project: Project::load(&store, &p.id)?, head: None, cert: cert.clone(), key: DeviceKey::from_bytes(key.to_bytes()) };
        let v0 = repo.save(&mut app, "import")?.unwrap();
        let old_root = project::load_version(&store, &v0)?.tree()?;
        // join: everything
        let mut all = vec![];
        tree::reachable(&store, old_root, &mut all)?;
        let join_raw: usize = all.iter().map(|i| store.get(i).unwrap().len()).sum();
        let join_concat: Vec<u8> = all.iter().flat_map(|i| store.get(i).unwrap()).collect();
        // one small edit
        let k = app.photos.keys().nth(n / 2).unwrap().clone();
        app.photos.get_mut(&k).unwrap().state["develop"]["exposure"] = json!(0.4);
        let v1 = repo.save(&mut app, "edit")?.unwrap();
        let new_root = project::load_version(&store, &v1)?.tree()?;
        let mut missing = vec![];
        tree::missing(&store, new_root, old_root, &mut missing)?;
        missing.push(v1);
        let mut by_kind: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let mut raw = 0;
        let mut defl = 0;
        for id in &missing {
            let b = store.get(id)?;
            raw += b.len();
            defl += deflate(&b);
            let e = by_kind.entry(kind(&b)).or_default();
            e.0 += 1;
            e.1 += b.len();
        }
        // node deltas against the node the receiver already has at the same place
        let mut pairs = vec![];
        tree::delta_bases(&store, new_root, old_root, &mut pairs)?;
        let mut delta_bytes = 0;
        let mut delta_ok = 0;
        let mut paired: std::collections::BTreeSet<Id> = Default::default();
        for (nid, oid) in &pairs {
            let nb = store.get(nid)?;
            let sent = match oid.map(|o| store.get(&o)).transpose()? {
                Some(ob) => match tree::node_delta(&ob, &nb) {
                    Some(d) => {
                        let rebuilt = tree::apply_node_delta(&ob, &d)?;
                        assert_eq!(Id::of(&rebuilt), *nid, "a rebuilt node must hash to the same id");
                        delta_ok += 1;
                        d.len() + 32 // + the id of the base node
                    }
                    None => nb.len(),
                },
                None => nb.len(),
            };
            delta_bytes += sent;
            paired.insert(*nid);
        }
        let rest: usize = missing.iter().filter(|i| !paired.contains(*i)).map(|i| store.get(i).unwrap().len()).sum();
        let rest_defl: usize = missing.iter().filter(|i| !paired.contains(*i)).map(|i| deflate(&store.get(i).unwrap())).sum();
        println!(
            "{}",
            json!({
                "photos": n,
                "join_raw_bytes": join_raw, "join_deflate_bytes": deflate(&join_concat),
                "edit_objects": by_kind.iter().map(|(k, (c, b))| json!({"kind": k, "count": c, "bytes": b})).collect::<Vec<_>>(),
                "edit_raw_bytes": raw,
                "edit_deflate_each_bytes": defl,
                "edit_node_delta_bytes": delta_bytes + rest,
                "edit_node_delta_plus_deflate_bytes": delta_bytes + rest_defl,
                "node_deltas_verified": delta_ok,
            })
        );
    }
    Ok(())
}
