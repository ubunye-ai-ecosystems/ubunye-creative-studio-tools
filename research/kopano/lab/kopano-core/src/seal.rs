//! Sealed bundles (2026-10-18): a bundle anyone can carry (a taxi driver's phone, a stranger's USB
//! stick, a school server) but only the project's member devices can open. The carrier sees
//! random-looking bytes and cannot change, cut or reorder them without the receiver noticing.
//!
//! - One fresh random data key per bundle, used with XChaCha20-Poly1305 in 1 MiB chunks. Each
//!   chunk's nonce carries its position, and the last chunk is marked, so dropping, swapping or
//!   cutting chunks fails to open.
//! - The data key is wrapped once per recipient device. No new keys are needed: a device's ed25519
//!   key (already its identity) is used as an X25519 key, with one throw-away key per bundle.
//! - Inside is an ordinary bundle, so after opening everything is still verified by hash and
//!   signature; sealing adds secrecy from carriers, not trust in them.

use anyhow::{anyhow, bail, Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};

pub const MAGIC: &[u8] = b"KOPANO-SEALED-1\n";
const CHUNK: usize = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Header {
    v: u32,
    /// the bundle's throw-away public key (X25519)
    eph: String,
    /// random prefix of every chunk nonce
    nonce: String,
    recipients: Vec<Recipient>,
}

#[derive(Serialize, Deserialize)]
struct Recipient {
    /// the device's public key (hex), so a device finds its own entry
    device: String,
    wrapped: String,
}

fn kek(shared: &[u8; 32], eph: &[u8; 32], device: &[u8; 32]) -> XChaCha20Poly1305 {
    let mut m = Vec::with_capacity(96);
    m.extend(shared);
    m.extend(eph);
    m.extend(device);
    XChaCha20Poly1305::new(&blake3::derive_key("kopano sealed bundle v1 key wrap", &m).into())
}

fn chunk_nonce(prefix: &[u8; 16], i: u64) -> XNonce {
    let mut n = [0u8; 24];
    n[..16].copy_from_slice(prefix);
    n[16..].copy_from_slice(&i.to_le_bytes());
    n.into()
}

/// Seal `input` for the given member devices. Returns bytes written.
pub fn seal(input: &mut dyn Read, output: &mut dyn Write, recipients: &[VerifyingKey]) -> Result<u64> {
    if recipients.is_empty() {
        bail!("no recipients");
    }
    let mut rng = rand::thread_rng();
    let mut data_key = [0u8; 32];
    let mut eph_secret = [0u8; 32];
    let mut prefix = [0u8; 16];
    rng.fill_bytes(&mut data_key);
    rng.fill_bytes(&mut eph_secret);
    rng.fill_bytes(&mut prefix);
    let eph_pub = MontgomeryPoint::mul_base_clamped(eph_secret).to_bytes();
    let mut list = vec![];
    for r in recipients {
        let shared = r.to_montgomery().mul_clamped(eph_secret).to_bytes();
        let wrapped = kek(&shared, &eph_pub, r.as_bytes()).encrypt(&XNonce::default(), data_key.as_slice()).map_err(|_| anyhow!("wrap"))?;
        list.push(Recipient { device: hex::encode(r.as_bytes()), wrapped: hex::encode(wrapped) });
    }
    let hdr = Header { v: 1, eph: hex::encode(eph_pub), nonce: hex::encode(prefix), recipients: list };
    let mut written = 0u64;
    let head = [MAGIC, serde_json::to_string(&hdr)?.as_bytes(), b"\n"].concat();
    output.write_all(&head)?;
    written += head.len() as u64;
    let aead = XChaCha20Poly1305::new(&data_key.into());
    let mut buf = vec![0u8; CHUNK];
    let mut next = read_full(input, &mut buf)?;
    let mut i = 0u64;
    loop {
        let cur = buf[..next].to_vec();
        let n2 = read_full(input, &mut buf)?;
        let last = n2 == 0;
        let ct = aead.encrypt(&chunk_nonce(&prefix, i), Payload { msg: &cur, aad: &[last as u8] }).map_err(|_| anyhow!("seal"))?;
        output.write_all(&(ct.len() as u32).to_le_bytes())?;
        output.write_all(&ct)?;
        written += 4 + ct.len() as u64;
        if last {
            break;
        }
        next = n2;
        i += 1;
    }
    Ok(written)
}

/// Open a sealed bundle with this device's key. Fails, writing nothing usable, if the device is
/// not a recipient or a single byte was changed, cut or reordered.
pub fn open(input: &mut dyn BufRead, output: &mut dyn Write, me: &SigningKey) -> Result<u64> {
    let mut magic = vec![0u8; MAGIC.len()];
    input.read_exact(&mut magic)?;
    if magic != MAGIC {
        bail!("not a sealed Kopano bundle");
    }
    let mut line = String::new();
    input.take(1 << 20).read_line(&mut line)?;
    let hdr: Header = serde_json::from_str(&line).context("damaged sealed bundle header")?;
    let my = me.verifying_key();
    let entry = hdr.recipients.iter().find(|r| r.device == hex::encode(my.as_bytes())).context("this device is not one of the bundle's recipients")?;
    let eph: [u8; 32] = hex::decode(&hdr.eph)?.try_into().map_err(|_| anyhow!("header"))?;
    let prefix: [u8; 16] = hex::decode(&hdr.nonce)?.try_into().map_err(|_| anyhow!("header"))?;
    let shared = MontgomeryPoint(eph).mul_clamped(me.to_scalar_bytes()).to_bytes();
    let key = kek(&shared, &eph, my.as_bytes()).decrypt(&XNonce::default(), hex::decode(&entry.wrapped)?.as_slice()).map_err(|_| anyhow!("the bundle's key does not open: damaged, or sealed for someone else"))?;
    let aead = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("key"))?;
    let (mut i, mut total) = (0u64, 0u64);
    loop {
        let mut len = [0u8; 4];
        input.read_exact(&mut len).map_err(|_| anyhow!("sealed bundle is cut short"))?;
        let len = u32::from_le_bytes(len) as usize;
        if len > CHUNK + 16 {
            bail!("damaged sealed bundle");
        }
        let mut ct = vec![0u8; len];
        input.read_exact(&mut ct).map_err(|_| anyhow!("sealed bundle is cut short"))?;
        // try as a middle chunk first, then as the last one
        let n = chunk_nonce(&prefix, i);
        if let Ok(pt) = aead.decrypt(&n, Payload { msg: &ct, aad: &[0] }) {
            output.write_all(&pt)?;
            total += pt.len() as u64;
            i += 1;
            continue;
        }
        let pt = aead.decrypt(&n, Payload { msg: &ct, aad: &[1] }).map_err(|_| anyhow!("sealed bundle was changed or reordered"))?;
        output.write_all(&pt)?;
        total += pt.len() as u64;
        let mut extra = [0u8; 1];
        if input.read(&mut extra)? != 0 {
            bail!("sealed bundle has extra bytes after its end");
        }
        return Ok(total);
    }
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = r.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(size: usize) {
        let mut rng = rand::thread_rng();
        let (a, b, stranger) = (SigningKey::generate(&mut rng), SigningKey::generate(&mut rng), SigningKey::generate(&mut rng));
        let mut plain = vec![0u8; size];
        rng.fill_bytes(&mut plain);
        let mut sealed = vec![];
        seal(&mut plain.as_slice(), &mut sealed, &[a.verifying_key(), b.verifying_key()]).unwrap();
        for k in [&a, &b] {
            let mut out = vec![];
            open(&mut sealed.as_slice(), &mut out, k).unwrap();
            assert_eq!(out, plain);
        }
        assert!(open(&mut sealed.as_slice(), &mut vec![], &stranger).is_err(), "a stranger opened it");
        // the carrier cannot read: the plaintext does not appear in the sealed bytes
        if size >= 64 {
            assert!(!sealed.windows(32).any(|w| w == &plain[..32]));
        }
        // one flipped byte anywhere in the body, a cut, or a dropped chunk: refused
        let body = MAGIC.len() + sealed[MAGIC.len()..].iter().position(|&c| c == b'\n').unwrap() + 1;
        for at in [body + 5, sealed.len() / 2, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 1;
            assert!(open(&mut bad.as_slice(), &mut vec![], &a).is_err(), "flip at {at} accepted");
        }
        assert!(open(&mut &sealed[..sealed.len() - 10], &mut vec![], &a).is_err(), "cut accepted");
    }

    #[test]
    fn sealed_bundle_small_and_multi_chunk() {
        roundtrip(0);
        roundtrip(100);
        roundtrip(3 * CHUNK + 777);
    }

    #[test]
    fn dropping_a_whole_chunk_is_refused() {
        let mut rng = rand::thread_rng();
        let a = SigningKey::generate(&mut rng);
        let plain = vec![7u8; 3 * CHUNK];
        let mut sealed = vec![];
        seal(&mut plain.as_slice(), &mut sealed, &[a.verifying_key()]).unwrap();
        let body = MAGIC.len() + sealed[MAGIC.len()..].iter().position(|&c| c == b'\n').unwrap() + 1;
        let first_len = u32::from_le_bytes(sealed[body..body + 4].try_into().unwrap()) as usize;
        let mut cut = sealed[..body].to_vec();
        cut.extend(&sealed[body + 4 + first_len..]); // chunk 0 removed
        assert!(open(&mut cut.as_slice(), &mut vec![], &a).is_err());
        let mut out = vec![];
        open(&mut sealed.as_slice(), &mut out, &a).unwrap();
        assert_eq!(out.len(), plain.len());
    }
}
