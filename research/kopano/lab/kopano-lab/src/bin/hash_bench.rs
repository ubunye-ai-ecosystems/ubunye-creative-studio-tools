//! Experiment E3: is hashing originals a bottleneck? Streams files through BLAKE3 with a fixed
//! 1 MiB buffer (bounded memory, as a real importer would) and reports MB/s. Run once on files
//! not read since boot (cold, disk-bound) and again immediately (warm, CPU-bound).
//!
//! Usage: hash_bench <file>...

use anyhow::Result;
use std::io::Read;
use std::time::Instant;

fn main() -> Result<()> {
    println!("file,bytes,ms,mb_per_s,peak_buffer_bytes");
    for path in std::env::args().skip(1) {
        let mut f = std::fs::File::open(&path)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut h = blake3::Hasher::new();
        let mut n = 0u64;
        let t = Instant::now();
        loop {
            let r = f.read(&mut buf)?;
            if r == 0 {
                break;
            }
            h.update(&buf[..r]);
            n += r as u64;
        }
        let _ = h.finalize();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("{path},{n},{ms:.0},{:.0},{}", n as f64 / 1e6 / (ms / 1000.0), buf.len());
    }
    Ok(())
}
