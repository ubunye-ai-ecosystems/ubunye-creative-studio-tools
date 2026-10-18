//! The compact sync payload (E19/E20): versions oldest first, changed tree nodes as deltas from the
//! node at the same place in the parent tree, everything deflated. It arrives from another device,
//! so decoding has hard limits (security pass, 2026-10-18): before this, a sender could claim the
//! payload unpacks to 1 GiB and the receiver reserved that much memory.

use crate::project;
use crate::store::{Id, ObjectStore};
use crate::tree;
use anyhow::{anyhow, bail, Context, Result};

/// Largest unpacked payload accepted. A small edit is about 1 KB; joining a 100k-photo project
/// is about 10 MB. Bigger catch-ups come in several payloads.
pub const MAX_RAW: u64 = 64 << 20;

pub struct Decoded {
    pub objects: u64,
    /// New versions, in the order received (oldest first).
    pub versions: Vec<Id>,
}

/// Unpack and store every record. Each object is stored under the hash of its bytes, so nothing
/// can be stored under a false name; versions are only listed here, the caller verifies them.
pub fn decode_compact(store: &dyn ObjectStore, z: &[u8], declared_raw: u64) -> Result<Decoded> {
    if declared_raw > MAX_RAW {
        bail!("payload of {declared_raw} bytes refused (limit {MAX_RAW})");
    }
    let payload = miniz_oxide::inflate::decompress_to_vec_with_limit(z, declared_raw as usize).map_err(|e| anyhow!("bad compressed payload: {e:?}"))?;
    let mut at = 0usize;
    let mut out = Decoded { objects: 0, versions: vec![] };
    while at < payload.len() {
        let kind = payload[at];
        let len = u32::from_le_bytes(payload.get(at + 1..at + 5).context("truncated record")?.try_into()?) as usize;
        let end = (at + 5).checked_add(len).context("record too long")?;
        let data = payload.get(at + 5..end).context("truncated record")?;
        at = end;
        let b = match kind {
            0 => data.to_vec(),
            1 => {
                let base = Id(data.get(..32).context("delta without base")?.try_into()?);
                tree::apply_node_delta(&store.get(&base)?, &data[32..])?
            }
            _ => bail!("unknown record kind {kind}"),
        };
        let id = Id::of(&b);
        store.put_verified(&id, &b)?;
        out.objects += 1;
        if b.first() == Some(&b'{') && serde_json::from_slice::<project::Version>(&b).is_ok() {
            out.versions.push(id);
        }
    }
    Ok(out)
}
