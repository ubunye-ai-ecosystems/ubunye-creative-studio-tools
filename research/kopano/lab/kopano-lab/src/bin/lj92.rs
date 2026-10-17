//! Precompression for lossless JPEG (ITU T.81 process 14, "LJ92"), the codec inside
//! Canon CR2, most DNGs and several other RAW formats. The same idea as precomp for
//! zlib streams: decode each stream, re-encode it with the file's own tables, and only
//! accept it when the bytes come back identical. The stronger compressor then works on
//! the plain samples, and `restore` rebuilds the camera's exact bytes.
//!
//!   lj92 strip <file> <rest.bin> <samples.u16> <meta.json>
//!   lj92 restore <rest.bin> <samples.u16> <meta.json> <out>
//!
//! `rest.bin` is the file with every verified entropy-coded segment cut out (headers stay).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone)]
struct Huff {
    // encode
    code: [u32; 256],
    size: [u8; 256],
    // decode
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [i32; 17],
    vals: Vec<u8>,
}

impl Huff {
    fn new(bits: &[u8; 17], vals: Vec<u8>) -> Result<Self> {
        let mut h = Huff { code: [0; 256], size: [0; 256], mincode: [0; 17], maxcode: [-1; 17], valptr: [0; 17], vals };
        let mut code = 0u32;
        let mut k = 0usize;
        for len in 1..=16 {
            let n = bits[len] as usize;
            if n > 0 {
                h.valptr[len] = k as i32;
                h.mincode[len] = code as i32;
                for _ in 0..n {
                    let sym = *h.vals.get(k).context("DHT shorter than its counts")? as usize;
                    h.code[sym] = code;
                    h.size[sym] = len as u8;
                    code += 1;
                    k += 1;
                }
                h.maxcode[len] = code as i32 - 1;
            }
            code <<= 1;
        }
        Ok(h)
    }
}

struct Frame {
    p: u8,
    y: usize,
    x: usize,
    nf: usize,
    ss: u8,
    ri: usize,
    td: Vec<usize>,
    tables: [Option<Huff>; 4],
}

fn be16(b: &[u8], i: usize) -> Result<usize> {
    Ok(((*b.get(i).context("eof")? as usize) << 8) | *b.get(i + 1).context("eof")? as usize)
}

/// Parse from SOI to the end of the SOS header. Returns the frame and the scan data start.
fn parse_header(b: &[u8], soi: usize) -> Result<(Frame, usize)> {
    let mut f = Frame { p: 0, y: 0, x: 0, nf: 0, ss: 0, ri: 0, td: vec![], tables: [None, None, None, None] };
    let mut pos = soi + 2;
    let mut ids: Vec<u8> = vec![];
    let mut seen_sof = false;
    loop {
        if *b.get(pos).context("eof")? != 0xFF {
            bail!("expected marker");
        }
        while b.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        let m = *b.get(pos + 1).context("eof")?;
        let len = be16(b, pos + 2)?;
        let seg = b.get(pos + 4..pos + 2 + len).context("segment past eof")?;
        match m {
            0xC4 => {
                let mut i = 0;
                while i < seg.len() {
                    let th = (seg[i] & 0x0F) as usize;
                    if th > 3 {
                        bail!("bad table id");
                    }
                    let mut bits = [0u8; 17];
                    bits[1..].copy_from_slice(seg.get(i + 1..i + 17).context("short DHT")?);
                    let n: usize = bits.iter().map(|&v| v as usize).sum();
                    let vals = seg.get(i + 17..i + 17 + n).context("short DHT")?.to_vec();
                    f.tables[th] = Some(Huff::new(&bits, vals)?);
                    i += 17 + n;
                }
            }
            0xC3 => {
                f.p = seg[0];
                f.y = be16(seg, 1)?;
                f.x = be16(seg, 3)?;
                f.nf = seg[5] as usize;
                for c in 0..f.nf {
                    ids.push(seg[6 + 3 * c]);
                    if seg[7 + 3 * c] != 0x11 {
                        bail!("subsampled components are not supported");
                    }
                }
                seen_sof = true;
            }
            0xC0..=0xCF if m != 0xC8 && m != 0xCC => bail!("not a lossless JPEG (SOF {m:02X})"),
            0xDD => f.ri = be16(seg, 0)?,
            0xDA => {
                if !seen_sof {
                    bail!("SOS before SOF3");
                }
                let ns = seg[0] as usize;
                if ns != f.nf {
                    bail!("non-interleaved scans are not supported");
                }
                for j in 0..ns {
                    let cid = seg[1 + 2 * j];
                    if ids.get(j) != Some(&cid) {
                        bail!("scan component order differs from frame");
                    }
                    f.td.push((seg[2 + 2 * j] >> 4) as usize);
                }
                f.ss = seg[1 + 2 * ns];
                let pt = seg[3 + 2 * ns] & 0x0F;
                if pt != 0 || !(1..=7).contains(&f.ss) {
                    bail!("point transform or predictor not supported");
                }
                for &t in &f.td {
                    if f.tables[t].is_none() {
                        bail!("missing Huffman table");
                    }
                }
                if f.x == 0 || f.y == 0 || f.p == 0 || f.p > 16 {
                    bail!("bad frame size");
                }
                return Ok((f, pos + 2 + len));
            }
            0xD8 | 0xD9 => bail!("unexpected SOI/EOI"),
            _ => {}
        }
        pos += 2 + len;
    }
}

/// End of the entropy-coded segment: the first marker that is not a stuffed byte or RST.
fn segment_end(b: &[u8], start: usize, ri: usize) -> usize {
    let mut i = start;
    while i + 1 < b.len() {
        if b[i] == 0xFF {
            let n = b[i + 1];
            if n == 0x00 || n == 0xFF || (ri > 0 && (0xD0..=0xD7).contains(&n)) {
                i += if n == 0xFF { 1 } else { 2 };
                continue;
            }
            return i;
        }
        i += 1;
    }
    b.len()
}

#[inline]
fn predict(s: &[u16], f: &Frame, row: usize, col: usize, c: usize, first_row: usize) -> i32 {
    let w = f.x * f.nf;
    let at = |r: usize, k: usize| s[r * w + k * f.nf + c] as i32;
    if row == first_row && col == 0 {
        return 1 << (f.p - 1);
    }
    if row == first_row {
        return at(row, col - 1);
    }
    if col == 0 {
        return at(row - 1, 0);
    }
    let (ra, rb, rc) = (at(row, col - 1), at(row - 1, col), at(row - 1, col - 1));
    match f.ss {
        1 => ra,
        2 => rb,
        3 => rc,
        4 => ra + rb - rc,
        5 => ra + ((rb - rc) >> 1),
        6 => rb + ((ra - rc) >> 1),
        _ => (ra + rb) >> 1,
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
}

impl<'a> Reader<'a> {
    #[inline]
    fn fill(&mut self) {
        while self.n <= 56 {
            let mut byte = 0u8;
            if self.pos < self.b.len() {
                let v = self.b[self.pos];
                if v == 0xFF {
                    if self.b.get(self.pos + 1) == Some(&0) {
                        self.pos += 2;
                        byte = 0xFF;
                    }
                } else {
                    self.pos += 1;
                    byte = v;
                }
            }
            self.acc |= (byte as u64) << (56 - self.n);
            self.n += 8;
        }
    }
    #[inline]
    fn bit(&mut self) -> u32 {
        if self.n == 0 {
            self.fill();
        }
        let v = (self.acc >> 63) as u32;
        self.acc <<= 1;
        self.n -= 1;
        v
    }
    #[inline]
    fn bits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        if self.n < k {
            self.fill();
        }
        let v = (self.acc >> (64 - k)) as u32;
        self.acc <<= k;
        self.n -= k;
        v
    }
    /// Drop buffered bits and skip an RSTn marker.
    fn restart(&mut self) -> Result<()> {
        // bytes already pulled into acc but not used belong to padding; find the marker from where we are
        let mut i = self.pos.saturating_sub(16);
        while i + 1 < self.b.len() && !(self.b[i] == 0xFF && (0xD0..=0xD7).contains(&self.b[i + 1])) {
            i += 1;
        }
        if i + 1 >= self.b.len() {
            bail!("restart marker not found");
        }
        self.pos = i + 2;
        self.acc = 0;
        self.n = 0;
        Ok(())
    }
}

fn decode(b: &[u8], start: usize, f: &Frame) -> Result<Vec<u16>> {
    let w = f.x * f.nf;
    let mut s = vec![0u16; w * f.y];
    let mut r = Reader { b, pos: start, acc: 0, n: 0 };
    let mut first_row = 0;
    let mut mcu = 0usize;
    for row in 0..f.y {
        for col in 0..f.x {
            if f.ri > 0 && mcu > 0 && mcu % f.ri == 0 {
                r.restart()?;
                first_row = row;
                if col != 0 {
                    bail!("restart inside a row is not supported");
                }
            }
            for c in 0..f.nf {
                let h = f.tables[f.td[c]].as_ref().unwrap();
                let mut code = r.bit() as i32;
                let mut l = 1;
                while l <= 16 && (h.maxcode[l] < 0 || code > h.maxcode[l]) {
                    code = (code << 1) | r.bit() as i32;
                    l += 1;
                }
                if l > 16 {
                    bail!("bad Huffman code at row {row}");
                }
                let ssss = h.vals[(h.valptr[l] + code - h.mincode[l]) as usize] as u32;
                let diff: i32 = match ssss {
                    0 => 0,
                    16 => 32768,
                    _ => {
                        let v = r.bits(ssss) as i32;
                        if v < (1 << (ssss - 1)) { v - (1 << ssss) + 1 } else { v }
                    }
                };
                let p = predict(&s, f, row, col, c, first_row);
                s[row * w + col * f.nf + c] = (p + diff) as u16;
            }
            mcu += 1;
        }
    }
    Ok(s)
}

struct Writer {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Writer {
    #[inline]
    fn put(&mut self, v: u32, k: u32) {
        if k == 0 {
            return;
        }
        self.acc = (self.acc << k) | (v as u64 & ((1u64 << k) - 1));
        self.n += k;
        while self.n >= 8 {
            let byte = (self.acc >> (self.n - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
    }
    fn align(&mut self) {
        if self.n > 0 {
            let k = 8 - self.n;
            self.put((1 << k) - 1, k);
        }
    }
}

fn encode(s: &[u16], f: &Frame) -> Vec<u8> {
    let w = f.x * f.nf;
    let mut wr = Writer { out: Vec::with_capacity(s.len() * 2), acc: 0, n: 0 };
    let mut first_row = 0;
    let mut mcu = 0usize;
    let mut rst = 0u8;
    for row in 0..f.y {
        for col in 0..f.x {
            if f.ri > 0 && mcu > 0 && mcu % f.ri == 0 {
                wr.align();
                wr.out.push(0xFF);
                wr.out.push(0xD0 + rst);
                rst = (rst + 1) & 7;
                first_row = row;
            }
            for c in 0..f.nf {
                let h = f.tables[f.td[c]].as_ref().unwrap();
                let p = predict(s, f, row, col, c, first_row);
                let d = ((s[row * w + col * f.nf + c] as i32 - p) & 0xFFFF) as u16;
                if d == 0x8000 {
                    wr.put(h.code[16], h.size[16] as u32);
                    continue;
                }
                let ds = d as i16 as i32;
                let mag = ds.unsigned_abs();
                let ssss = 32 - mag.leading_zeros();
                wr.put(h.code[ssss as usize], h.size[ssss as usize] as u32);
                let extra = if ds < 0 { (ds - 1) as u32 } else { ds as u32 };
                wr.put(extra, ssss);
            }
            mcu += 1;
        }
    }
    wr.align();
    wr.out
}

#[derive(Serialize, Deserialize)]
struct Stream {
    /// where the entropy segment goes back into rest.bin
    at_in_rest: usize,
    /// where its header (SOI) starts in rest.bin
    soi_in_rest: usize,
    seg_len: usize,
    sample_offset: usize,
    samples: usize,
    width: usize,
    height: usize,
    components: usize,
    predictor: u8,
    precision: u8,
    restart: usize,
}

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    file_len: usize,
    streams: Vec<Stream>,
    rejected: Vec<String>,
}

fn strip(path: &str, rest_p: &str, samples_p: &str, meta_p: &str) -> Result<()> {
    let b = std::fs::read(path)?;
    let t0 = Instant::now();
    let mut meta = Meta { file_len: b.len(), ..Default::default() };
    let mut rest: Vec<u8> = Vec::with_capacity(b.len());
    let mut samples: Vec<u16> = vec![];
    let mut copied = 0usize;
    let mut i = 0usize;
    while i + 3 < b.len() {
        if !(b[i] == 0xFF && b[i + 1] == 0xD8 && b[i + 2] == 0xFF) {
            i += 1;
            continue;
        }
        let (f, start) = match parse_header(&b, i) {
            Ok(v) => v,
            Err(_) => {
                i += 2;
                continue;
            }
        };
        let end = segment_end(&b, start, f.ri);
        let got = decode(&b, start, &f).and_then(|s| {
            let e = encode(&s, &f);
            if e[..] == b[start..end] { Ok(s) } else { bail!("re-encode differs ({} vs {} bytes)", e.len(), end - start) }
        });
        match got {
            Ok(s) => {
                rest.extend_from_slice(&b[copied..start]);
                let soi_in_rest = rest.len() - (start - i);
                meta.streams.push(Stream {
                    at_in_rest: rest.len(),
                    soi_in_rest,
                    seg_len: end - start,
                    sample_offset: samples.len(),
                    samples: s.len(),
                    width: f.x * f.nf,
                    height: f.y,
                    components: f.nf,
                    predictor: f.ss,
                    precision: f.p,
                    restart: f.ri,
                });
                samples.extend_from_slice(&s);
                copied = end;
                i = end;
            }
            Err(e) => {
                meta.rejected.push(format!("at {i}: {e}"));
                i = end.max(i + 2);
            }
        }
    }
    rest.extend_from_slice(&b[copied..]);
    std::fs::write(rest_p, &rest)?;
    let mut sb = Vec::with_capacity(samples.len() * 2);
    for v in &samples {
        sb.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(samples_p, &sb)?;
    std::fs::write(meta_p, serde_json::to_vec_pretty(&meta)?)?;
    let cut: usize = meta.streams.iter().map(|s| s.seg_len).sum();
    println!(
        "{}",
        serde_json::json!({"streams": meta.streams.len(), "rejected": meta.rejected.len(), "entropy_bytes": cut,
            "file_bytes": b.len(), "secs": t0.elapsed().as_secs_f64()})
    );
    Ok(())
}

fn restore(rest_p: &str, samples_p: &str, meta_p: &str, out_p: &str) -> Result<()> {
    let rest = std::fs::read(rest_p)?;
    let sb = std::fs::read(samples_p)?;
    let samples: Vec<u16> = sb.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let meta: Meta = serde_json::from_slice(&std::fs::read(meta_p)?)?;
    let t0 = Instant::now();
    let mut out = Vec::with_capacity(meta.file_len);
    let mut copied = 0usize;
    for st in &meta.streams {
        let (f, start) = parse_header(&rest, st.soi_in_rest)?;
        if start != st.at_in_rest {
            bail!("header length mismatch");
        }
        out.extend_from_slice(&rest[copied..st.at_in_rest]);
        let e = encode(&samples[st.sample_offset..st.sample_offset + st.samples], &f);
        if e.len() != st.seg_len {
            bail!("rebuilt segment has the wrong length");
        }
        out.extend_from_slice(&e);
        copied = st.at_in_rest;
    }
    out.extend_from_slice(&rest[copied..]);
    std::fs::write(out_p, &out)?;
    println!("{}", serde_json::json!({"bytes": out.len(), "secs": t0.elapsed().as_secs_f64()}));
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("strip") if a.len() == 6 => strip(&a[2], &a[3], &a[4], &a[5]),
        Some("restore") if a.len() == 6 => restore(&a[2], &a[3], &a[4], &a[5]),
        _ => bail!("usage: lj92 strip <file> <rest> <samples> <meta> | lj92 restore <rest> <samples> <meta> <out>"),
    }
}
