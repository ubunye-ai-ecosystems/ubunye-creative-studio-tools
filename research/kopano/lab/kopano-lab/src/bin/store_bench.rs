//! Experiment E7: one file per object (Git loose objects, the prototype's FsStore) vs one embedded
//! ACID database file (redb, already in iroh-blobs' dependency tree) for Kopano's small objects.
//! Writes N objects of ~330 bytes (the measured average item size), then reads them all back.
//!
//! Usage: store_bench <dir> <n>

use anyhow::Result;
use kopano_core::store::{FsStore, Id, ObjectStore};
use redb::{Database, ReadableDatabase, TableDefinition};
use std::time::Instant;

const OBJECTS: TableDefinition<&[u8; 32], &[u8]> = TableDefinition::new("objects");

fn obj(i: usize) -> Vec<u8> {
    format!("{{\"kind\":\"bench\",\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(290)).into_bytes()
}

fn main() -> Result<()> {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().unwrap_or_else(|| ".".into()));
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let objs: Vec<Vec<u8>> = (0..n).map(obj).collect();
    let ids: Vec<Id> = objs.iter().map(|o| Id::of(o)).collect();
    println!("store,n,write_ms,read_ms,write_objs_per_s,read_objs_per_s,disk_bytes");

    // One file per object.
    let fs = FsStore::open(dir.join("fs"))?;
    let t = Instant::now();
    for o in &objs {
        fs.put(o)?;
    }
    let w = t.elapsed().as_secs_f64();
    let t = Instant::now();
    for id in &ids {
        let _ = fs.get(id)?;
    }
    let r = t.elapsed().as_secs_f64();
    println!("files,{n},{:.0},{:.0},{:.0},{:.0},{}", w * 1e3, r * 1e3, n as f64 / w, n as f64 / r, dir_size(&dir.join("fs")));

    // redb: one transaction per sync batch (what a pull would do), durable commit.
    let db = Database::create(dir.join("kopano.redb"))?;
    let t = Instant::now();
    for chunk in objs.chunks(5_000) {
        let tx = db.begin_write()?;
        {
            let mut table = tx.open_table(OBJECTS)?;
            for o in chunk {
                table.insert(&Id::of(o).0, o.as_slice())?;
            }
        }
        tx.commit()?;
    }
    let w = t.elapsed().as_secs_f64();
    let t = Instant::now();
    {
        let tx = db.begin_read()?;
        let table = tx.open_table(OBJECTS)?;
        for id in &ids {
            let v = table.get(&id.0)?.ok_or_else(|| anyhow::anyhow!("missing"))?;
            // Verify on read, as FsStore does.
            if Id::of(v.value()) != *id {
                anyhow::bail!("damaged");
            }
        }
    }
    let r = t.elapsed().as_secs_f64();
    drop(db);
    println!("redb-batched,{n},{:.0},{:.0},{:.0},{:.0},{}", w * 1e3, r * 1e3, n as f64 / w, n as f64 / r, std::fs::metadata(dir.join("kopano.redb"))?.len());

    // redb: one durable transaction per object (worst case: an app saving one edit at a time).
    let db = Database::create(dir.join("kopano1.redb"))?;
    let m = n.min(2_000);
    let t = Instant::now();
    for o in objs.iter().take(m) {
        let tx = db.begin_write()?;
        {
            let mut table = tx.open_table(OBJECTS)?;
            table.insert(&Id::of(o).0, o.as_slice())?;
        }
        tx.commit()?;
    }
    let w = t.elapsed().as_secs_f64();
    println!("redb-per-object,{m},{:.0},,{:.0},,", w * 1e3, m as f64 / w);
    Ok(())
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut s = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            s += if path.is_dir() { dir_size(&path) } else { e.metadata().map(|m| m.len()).unwrap_or(0) };
        }
    }
    s
}
