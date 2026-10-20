//! How well do raw Photo tiles pack? Usage: tile_pack_bench <folder of raw 256x256 RGBA tiles>.
//! Compares zstd (Fastest, the encoder Photo bundles use) with two pixel filters before zstd.
use std::io::Read;
use std::time::Instant;
fn filt(raw: &[u8]) -> Vec<u8> {
    // planes R,G,B,A; each row: left delta
    let n = raw.len() / 4;
    let w = 256usize;
    let mut out = vec![0u8; raw.len()];
    for c in 0..4 {
        for i in 0..n {
            let v = raw[i * 4 + c];
            let left = if i % w == 0 { if i >= w { raw[(i - w) * 4 + c] } else { 0 } } else { raw[(i - 1) * 4 + c] };
            out[c * n + i] = v.wrapping_sub(left);
        }
    }
    out
}
fn filt2(raw: &[u8]) -> Vec<u8> {
    // planes, G first; R-G and B-G decorrelation; then paeth-ish (avg of left and up) delta
    let n = raw.len() / 4;
    let w = 256usize;
    let px = |i: usize, c: usize| -> u8 {
        let g = raw[i * 4 + 1];
        match c { 0 => raw[i*4].wrapping_sub(g), 2 => raw[i*4+2].wrapping_sub(g), _ => raw[i * 4 + c] }
    };
    let mut out = vec![0u8; raw.len()];
    for (pi, c) in [1usize, 0, 2, 3].iter().enumerate() {
        for i in 0..n {
            let x = i % w;
            let a = if x > 0 { px(i - 1, *c) as i32 } else { 0 };
            let b = if i >= w { px(i - w, *c) as i32 } else { 0 };
            let cc = if x > 0 && i >= w { px(i - w - 1, *c) as i32 } else { 0 };
            let p = a + b - cc;
            let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - cc).abs());
            let pred = if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { cc };
            out[pi * n + i] = px(i, *c).wrapping_sub(pred as u8);
        }
    }
    out
}
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut tot = [0usize; 5];
    let mut t = [0u128; 4];
    let mut files = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let raw = std::fs::read(&p).unwrap();
        if raw.len() != 262144 { continue; }
        files += 1;
        tot[0] += raw.len();
        let s = Instant::now();
        let z = ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        t[0] += s.elapsed().as_micros();
        tot[1] += z.len();
        let s = Instant::now();
        let mut back = Vec::new();
        ruzstd::decoding::StreamingDecoder::new(z.as_slice()).unwrap().read_to_end(&mut back).unwrap();
        t[1] += s.elapsed().as_micros();
        assert_eq!(back, raw);
        let s = Instant::now();
        let f = filt(&raw);
        let z = ruzstd::encoding::compress_to_vec(f.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        t[2] += s.elapsed().as_micros();
        tot[2] += z.len();
        let s = Instant::now();
        let f = filt2(&raw);
        let z = ruzstd::encoding::compress_to_vec(f.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        t[3] += s.elapsed().as_micros();
        tot[3] += z.len();
    }
    println!("{files} tiles raw {} MB; zstd-fastest {} MB ({} us/tile enc, {} us dec); planes+left {} MB ({} us); G-decor+paeth {} MB ({} us)",
        tot[0] / 1000000, tot[1] as f64 / 1e6, t[0] / files as u128, t[1] / files as u128, tot[2] as f64 / 1e6, t[2] / files as u128, tot[3] as f64 / 1e6, t[3] / files as u128);
}
