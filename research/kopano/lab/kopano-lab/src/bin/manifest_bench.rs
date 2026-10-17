//! Experiment E4: does a version of a huge catalogue stay small and fast?
//! Compares a flat manifest (one object listing every item, the first draft's design) with the
//! hash trie in kopano-core, at 10k / 100k / 1M items. Item states are ~1.5 KB JSON, like a
//! photo editor's develop settings. Measures: initial build, bytes of new objects for a one-item
//! edit and a 1% batch edit, time to diff two versions, peak object count. In-memory store, so
//! this isolates data-structure cost from disk speed.
//!
//! Usage: manifest_bench [n ...]

use anyhow::Result;
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::tree;
use serde_json::json;
use std::time::Instant;

fn item(i: usize, exposure: f64) -> Vec<u8> {
    let mut develop = serde_json::Map::new();
    for s in 0..40 {
        develop.insert(format!("setting_{s:02}"), json!(0.0));
    }
    develop.insert("exposure".into(), json!(exposure));
    serde_json::to_vec(&json!({"kind": "bench.photo", "lane": "develop", "schema": 1, "state": {"develop": develop, "rating": i % 6}, "assets": [{"hash": blake3::hash(&i.to_le_bytes()).to_hex().to_string(), "size": 50_000_000, "role": "original"}]})).unwrap_or_default()
}

fn flat_manifest(entries: &[(String, Id)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 64);
    for (k, v) in entries {
        out.extend(k.as_bytes());
        out.push(0);
        out.extend(v.0);
    }
    out
}

fn main() -> Result<()> {
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![10_000, 100_000, 1_000_000] } else { sizes };
    println!("items,design,build_ms,item_state_bytes_avg,edit1_new_bytes,edit1_ms,edit1pct_new_bytes,edit1pct_ms,diff1_ms,diff1pct_ms,objects");
    for n in sizes {
        let s = MemStore::default();
        let t = Instant::now();
        let mut entries: Vec<(String, Id)> = Vec::with_capacity(n);
        let mut state_bytes = 0usize;
        for i in 0..n {
            let b = item(i, 0.0);
            state_bytes += b.len();
            entries.push((format!("photos/IMG_{i:07}.ARW"), s.put(&b)?));
        }
        let items_ms = t.elapsed().as_millis();

        // Trie
        let t = Instant::now();
        let root = tree::update(&s, None, entries.iter().map(|(k, v)| (k.clone(), Some(*v))).collect())?;
        let build_ms = t.elapsed().as_millis() + items_ms;
        let objects = s.len();

        s.reset_counter();
        let t = Instant::now();
        let e1 = s.put(&item(42, 0.3))?;
        let root1 = tree::update(&s, root, vec![(format!("photos/IMG_{:07}.ARW", 42), Some(e1))])?;
        let edit1_ms = t.elapsed().as_secs_f64() * 1000.0;
        let edit1_bytes = s.bytes_written();

        s.reset_counter();
        let t = Instant::now();
        let mut batch = Vec::new();
        for i in (0..n).step_by(100) {
            batch.push((format!("photos/IMG_{i:07}.ARW"), Some(s.put(&item(i, 0.5))?)));
        }
        let root2 = tree::update(&s, root, batch)?;
        let editp_ms = t.elapsed().as_secs_f64() * 1000.0;
        let editp_bytes = s.bytes_written();

        let t = Instant::now();
        let d1 = tree::diff(&s, root, root1)?;
        let diff1_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let dp = tree::diff(&s, root, root2)?;
        let diffp_ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(d1.len(), 1);
        assert_eq!(dp.len(), n.div_ceil(100));
        println!("{n},trie,{build_ms},{},{edit1_bytes},{edit1_ms:.2},{editp_bytes},{editp_ms:.1},{diff1_ms:.2},{diffp_ms:.1},{objects}", state_bytes / n);

        // Flat manifest: every save writes the whole list.
        let t = Instant::now();
        let flat = flat_manifest(&entries);
        let flat_build = t.elapsed().as_millis() + items_ms;
        let t = Instant::now();
        let mut e2 = entries.clone();
        e2[42].1 = e1;
        let flat1 = flat_manifest(&e2);
        let _ = Id::of(&flat1);
        let f_edit1_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let changed = entries.iter().zip(&e2).filter(|(a, b)| a.1 != b.1).count();
        let f_diff_ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(changed, 1);
        println!("{n},flat,{flat_build},{},{},{f_edit1_ms:.2},{},{f_edit1_ms:.1},{f_diff_ms:.2},{f_diff_ms:.1},{}", state_bytes / n, flat1.len() + 1500, flat.len() + n / 100 * 1500, n + 1);
    }
    Ok(())
}
