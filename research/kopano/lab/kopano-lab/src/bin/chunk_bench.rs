//! Experiment E2: how many bytes must move when a media file changes, under whole-file,
//! fixed-size and content-defined chunking (FastCDC), for realistic changes to real files.
//!
//! Usage: chunk_bench <scenario-dir>
//! The dir holds pairs prepared by `prepare_scenarios.ps1`/the lab notes: `<name>.a` (what the
//! receiver has) and `<name>.b` (the new file). Prints one CSV row per scenario and method.
//! Measures: bytes to transfer, chunk count (metadata overhead), and chunking + hashing speed.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::time::Instant;

fn whole(data: &[u8]) -> Vec<([u8; 32], usize)> {
    vec![(*blake3::hash(data).as_bytes(), data.len())]
}

fn fixed(data: &[u8], size: usize) -> Vec<([u8; 32], usize)> {
    data.chunks(size).map(|c| (*blake3::hash(c).as_bytes(), c.len())).collect()
}

fn cdc(data: &[u8], avg: u32) -> Vec<([u8; 32], usize)> {
    fastcdc::v2020::FastCDC::new(data, avg / 4, avg, avg * 4)
        .map(|c| {
            let s = &data[c.offset..c.offset + c.length];
            (*blake3::hash(s).as_bytes(), c.length)
        })
        .collect()
}

fn main() -> Result<()> {
    let dir = std::env::args().nth(1).context("usage: chunk_bench <scenario-dir>")?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".a")).map(String::from))
        .collect();
    names.sort();
    println!("scenario,method,new_file_bytes,bytes_to_send,send_pct,chunks_in_new,index_bytes,chunk_ms,mb_per_s");
    for name in names {
        let a = std::fs::read(format!("{dir}/{name}.a"))?;
        let b = std::fs::read(format!("{dir}/{name}.b"))?;
        let methods: Vec<(&str, Box<dyn Fn(&[u8]) -> Vec<([u8; 32], usize)>>)> = vec![
            ("whole-file", Box::new(whole)),
            ("fixed-64K", Box::new(|d| fixed(d, 64 << 10))),
            ("fixed-1M", Box::new(|d| fixed(d, 1 << 20))),
            ("cdc-64K", Box::new(|d| cdc(d, 64 << 10))),
            ("cdc-1M", Box::new(|d| cdc(d, 1 << 20))),
        ];
        for (m, f) in methods {
            let have: HashSet<[u8; 32]> = f(&a).into_iter().map(|c| c.0).collect();
            let t = Instant::now();
            let new = f(&b);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let mut sent = HashSet::new();
            let send: usize = new.iter().filter(|c| !have.contains(&c.0) && sent.insert(c.0)).map(|c| c.1).sum();
            println!(
                "{name},{m},{},{send},{:.2},{},{},{ms:.0},{:.0}",
                b.len(),
                100.0 * send as f64 / b.len().max(1) as f64,
                new.len(),
                new.len() * 40,
                b.len() as f64 / 1e6 / (ms / 1000.0).max(1e-9)
            );
        }
    }
    Ok(())
}
