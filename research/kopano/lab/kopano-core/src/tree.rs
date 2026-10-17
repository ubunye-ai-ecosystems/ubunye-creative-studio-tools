//! The project manifest: item key -> item object id, stored as a canonical hash trie.
//!
//! Why not one flat list (the first draft)? A flat manifest of 1,000,000 items is tens of MB, and
//! every save would rewrite all of it. Here each node holds at most `MAX_LEAF` entries; a branch
//! splits on one byte of BLAKE3(key). A one-item edit rewrites only the nodes on one path, and
//! comparing two versions skips every subtree whose id is equal (Git trees, Merkle DAG idea).
//!
//! Canonical: a subtree is a leaf if and only if it holds `MAX_LEAF` or fewer entries. So the shape
//! depends only on the contents, and two peers that hold the same items compute the same root.

use crate::store::{Id, ObjectStore};
use anyhow::{bail, Result};
use std::collections::BTreeMap;

pub const MAX_LEAF: usize = 64;
const MAX_DEPTH: usize = 31;

enum Node {
    Leaf(BTreeMap<String, Id>),
    Branch(BTreeMap<u8, (Id, u64)>),
}

fn key_hash(key: &str) -> [u8; 32] {
    *blake3::hash(key.as_bytes()).as_bytes()
}

fn encode(n: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    match n {
        Node::Leaf(m) => {
            out.push(b'L');
            out.extend((m.len() as u32).to_le_bytes());
            for (k, v) in m {
                out.extend((k.len() as u32).to_le_bytes());
                out.extend(k.as_bytes());
                out.extend(v.0);
            }
        }
        Node::Branch(m) => {
            out.push(b'B');
            out.extend((m.len() as u16).to_le_bytes());
            for (b, (id, n)) in m {
                out.push(*b);
                out.extend(id.0);
                out.extend(n.to_le_bytes());
            }
        }
    }
    out
}

fn take<'a>(buf: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = at.checked_add(n).filter(|e| *e <= buf.len());
    let Some(end) = end else { bail!("tree node is truncated") };
    let s = &buf[*at..end];
    *at = end;
    Ok(s)
}

fn decode(buf: &[u8]) -> Result<Node> {
    let mut at = 0;
    let tag = take(buf, &mut at, 1)?[0];
    match tag {
        b'L' => {
            let n = u32::from_le_bytes(take(buf, &mut at, 4)?.try_into()?) as usize;
            let mut m = BTreeMap::new();
            for _ in 0..n.min(MAX_LEAF * 2) {
                let kl = u32::from_le_bytes(take(buf, &mut at, 4)?.try_into()?) as usize;
                let k = String::from_utf8(take(buf, &mut at, kl)?.to_vec())?;
                let id = Id(take(buf, &mut at, 32)?.try_into()?);
                m.insert(k, id);
            }
            Ok(Node::Leaf(m))
        }
        b'B' => {
            let n = u16::from_le_bytes(take(buf, &mut at, 2)?.try_into()?) as usize;
            let mut m = BTreeMap::new();
            for _ in 0..n.min(256) {
                let b = take(buf, &mut at, 1)?[0];
                let id = Id(take(buf, &mut at, 32)?.try_into()?);
                let c = u64::from_le_bytes(take(buf, &mut at, 8)?.try_into()?);
                m.insert(b, (id, c));
            }
            Ok(Node::Branch(m))
        }
        _ => bail!("not a tree node"),
    }
}

fn load(store: &dyn ObjectStore, id: &Id) -> Result<Node> {
    decode(&store.get(id)?)
}

/// One change: set `key` to `value`, or remove it with `None`.
pub type Change = (String, Option<Id>);

/// Apply changes to a tree and return the new root (`None` = empty tree).
pub fn update(store: &dyn ObjectStore, root: Option<Id>, changes: Vec<Change>) -> Result<Option<Id>> {
    let with_hash: Vec<([u8; 32], String, Option<Id>)> =
        changes.into_iter().map(|(k, v)| (key_hash(&k), k, v)).collect();
    Ok(update_at(store, root, with_hash, 0)?.map(|(id, _)| id))
}

fn update_at(
    store: &dyn ObjectStore,
    node: Option<Id>,
    changes: Vec<([u8; 32], String, Option<Id>)>,
    depth: usize,
) -> Result<Option<(Id, u64)>> {
    if changes.is_empty() {
        return match node {
            None => Ok(None),
            Some(id) => Ok(Some((id, count(store, &id)?))),
        };
    }
    let current = match node {
        None => Node::Leaf(BTreeMap::new()),
        Some(id) => load(store, &id)?,
    };
    match current {
        Node::Leaf(mut m) => {
            for (_, k, v) in &changes {
                match v {
                    Some(v) => {
                        m.insert(k.clone(), *v);
                    }
                    None => {
                        m.remove(k);
                    }
                }
            }
            if m.is_empty() {
                return Ok(None);
            }
            if m.len() <= MAX_LEAF || depth >= MAX_DEPTH {
                let n = m.len() as u64;
                return Ok(Some((store.put(&encode(&Node::Leaf(m)))?, n)));
            }
            // Too big for one leaf: split by the next byte of the key hash.
            let all = m.into_iter().map(|(k, v)| (key_hash(&k), k, Some(v))).collect();
            build_branch(store, BTreeMap::new(), all, depth)
        }
        Node::Branch(children) => build_branch(store, children, changes, depth),
    }
}

fn build_branch(
    store: &dyn ObjectStore,
    mut children: BTreeMap<u8, (Id, u64)>,
    changes: Vec<([u8; 32], String, Option<Id>)>,
    depth: usize,
) -> Result<Option<(Id, u64)>> {
    let mut groups: BTreeMap<u8, Vec<([u8; 32], String, Option<Id>)>> = BTreeMap::new();
    for c in changes {
        groups.entry(c.0[depth]).or_default().push(c);
    }
    for (b, group) in groups {
        let child = children.get(&b).map(|(id, _)| *id);
        match update_at(store, child, group, depth + 1)? {
            Some(c) => {
                children.insert(b, c);
            }
            None => {
                children.remove(&b);
            }
        }
    }
    let total: u64 = children.values().map(|(_, n)| n).sum();
    if total == 0 {
        return Ok(None);
    }
    if total as usize <= MAX_LEAF {
        // Small enough again: collapse back to one leaf so the shape stays canonical.
        let mut m = BTreeMap::new();
        for (id, _) in children.values() {
            collect(store, id, &mut m)?;
        }
        return Ok(Some((store.put(&encode(&Node::Leaf(m)))?, total)));
    }
    Ok(Some((store.put(&encode(&Node::Branch(children)))?, total)))
}

fn count(store: &dyn ObjectStore, id: &Id) -> Result<u64> {
    Ok(match load(store, id)? {
        Node::Leaf(m) => m.len() as u64,
        Node::Branch(c) => c.values().map(|(_, n)| n).sum(),
    })
}

fn collect(store: &dyn ObjectStore, id: &Id, out: &mut BTreeMap<String, Id>) -> Result<()> {
    match load(store, id)? {
        Node::Leaf(m) => out.extend(m),
        Node::Branch(c) => {
            for (cid, _) in c.values() {
                collect(store, cid, out)?;
            }
        }
    }
    Ok(())
}

/// Every entry of a tree.
pub fn entries(store: &dyn ObjectStore, root: Option<Id>) -> Result<BTreeMap<String, Id>> {
    let mut out = BTreeMap::new();
    if let Some(r) = root {
        collect(store, &r, &mut out)?;
    }
    Ok(out)
}

pub fn get(store: &dyn ObjectStore, root: Option<Id>, key: &str) -> Result<Option<Id>> {
    let h = key_hash(key);
    let mut cur = root;
    let mut depth = 0;
    while let Some(id) = cur {
        match load(store, &id)? {
            Node::Leaf(m) => return Ok(m.get(key).copied()),
            Node::Branch(c) => {
                cur = c.get(&h[depth]).map(|(id, _)| *id);
                depth += 1;
            }
        }
    }
    Ok(None)
}

pub fn len(store: &dyn ObjectStore, root: Option<Id>) -> Result<u64> {
    match root {
        None => Ok(0),
        Some(r) => count(store, &r),
    }
}

/// Keys whose value differs between two trees: (key, in a, in b). Equal subtrees are skipped
/// without being read, so a small change between two huge versions is cheap to find.
pub fn diff(store: &dyn ObjectStore, a: Option<Id>, b: Option<Id>) -> Result<Vec<(String, Option<Id>, Option<Id>)>> {
    let mut out = Vec::new();
    diff_at(store, a, b, &mut out)?;
    Ok(out)
}

fn diff_at(
    store: &dyn ObjectStore,
    a: Option<Id>,
    b: Option<Id>,
    out: &mut Vec<(String, Option<Id>, Option<Id>)>,
) -> Result<()> {
    if a == b {
        return Ok(());
    }
    let na = a.map(|id| load(store, &id)).transpose()?;
    let nb = b.map(|id| load(store, &id)).transpose()?;
    match (na, nb) {
        (Some(Node::Branch(ca)), Some(Node::Branch(cb))) => {
            let keys: std::collections::BTreeSet<u8> = ca.keys().chain(cb.keys()).copied().collect();
            for k in keys {
                diff_at(store, ca.get(&k).map(|x| x.0), cb.get(&k).map(|x| x.0), out)?;
            }
        }
        _ => {
            // A leaf on either side (or one side empty): compare the flattened entries.
            let ea = entries(store, a)?;
            let eb = entries(store, b)?;
            for (k, va) in &ea {
                match eb.get(k) {
                    Some(vb) if vb == va => {}
                    other => out.push((k.clone(), Some(*va), other.copied())),
                }
            }
            for (k, vb) in &eb {
                if !ea.contains_key(k) {
                    out.push((k.clone(), None, Some(*vb)));
                }
            }
        }
    }
    Ok(())
}

/// Every object id reachable from a root (for sync: "what does the other side need").
pub fn reachable(store: &dyn ObjectStore, root: Option<Id>, out: &mut Vec<Id>) -> Result<()> {
    let Some(id) = root else { return Ok(()) };
    out.push(id);
    match load(store, &id)? {
        Node::Leaf(m) => out.extend(m.values().copied()),
        Node::Branch(c) => {
            for (cid, _) in c.values() {
                reachable(store, Some(*cid), out)?;
            }
        }
    }
    Ok(())
}

/// Object ids reachable from `new` that are not reachable from `old`, walking both trees in step
/// and skipping every subtree whose id is equal. This is what a peer holding `old` lacks, so a
/// server can send it all in one response instead of one round trip per tree level.
pub fn missing(store: &dyn ObjectStore, new: Option<Id>, old: Option<Id>, out: &mut Vec<Id>) -> Result<()> {
    let Some(n) = new else { return Ok(()) };
    if Some(n) == old {
        return Ok(());
    }
    out.push(n);
    let old_node = old.map(|o| load(store, &o)).transpose()?;
    match (load(store, &n)?, old_node) {
        (Node::Branch(cn), Some(Node::Branch(co))) => {
            for (b, (cid, _)) in cn {
                missing(store, Some(cid), co.get(&b).map(|x| x.0), out)?;
            }
        }
        (Node::Leaf(m), Some(Node::Leaf(o))) => {
            let have: std::collections::HashSet<Id> = o.values().copied().collect();
            out.extend(m.values().filter(|v| !have.contains(v)).copied());
        }
        (Node::Leaf(m), _) => out.extend(m.values().copied()),
        (Node::Branch(cn), _) => {
            for (cid, _) in cn.values() {
                reachable(store, Some(*cid), out)?;
            }
        }
    }
    Ok(())
}

/// The ids a node points to (children or item objects), for walking a remote tree level by level.
pub fn children(bytes: &[u8]) -> Result<Vec<Id>> {
    Ok(match decode(bytes)? {
        Node::Leaf(m) => m.into_values().collect(),
        Node::Branch(c) => c.into_values().map(|(id, _)| id).collect(),
    })
}

/// Whether `children()` of this node are tree nodes (branch) or item objects (leaf).
pub fn is_branch(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'B')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;

    fn val(i: u64) -> Id {
        Id::of(&i.to_le_bytes())
    }

    #[test]
    fn same_contents_same_root_regardless_of_order() {
        let s = MemStore::default();
        let a: Vec<Change> = (0..1000).map(|i| (format!("k{i}"), Some(val(i)))).collect();
        let mut b = a.clone();
        b.reverse();
        let ra = update(&s, None, a).unwrap();
        // Build b incrementally in two halves.
        let (b1, b2) = b.split_at(400);
        let rb = update(&s, None, b1.to_vec()).unwrap();
        let rb = update(&s, rb, b2.to_vec()).unwrap();
        assert_eq!(ra, rb);
        assert_eq!(len(&s, ra).unwrap(), 1000);
    }

    #[test]
    fn remove_collapses_back_to_canonical_shape() {
        let s = MemStore::default();
        let small: Vec<Change> = (0..10).map(|i| (format!("k{i}"), Some(val(i)))).collect();
        let r_small = update(&s, None, small).unwrap();
        let big: Vec<Change> = (0..500).map(|i| (format!("k{i}"), Some(val(i)))).collect();
        let r_big = update(&s, None, big).unwrap();
        let removals: Vec<Change> = (10..500).map(|i| (format!("k{i}"), None)).collect();
        let r_back = update(&s, r_big, removals).unwrap();
        assert_eq!(r_small, r_back);
    }

    #[test]
    fn diff_finds_exactly_the_changes() {
        let s = MemStore::default();
        let base: Vec<Change> = (0..5000).map(|i| (format!("k{i}"), Some(val(i)))).collect();
        let r1 = update(&s, None, base).unwrap();
        let r2 = update(
            &s,
            r1,
            vec![("k7".into(), Some(val(99_999))), ("k4000".into(), None), ("new".into(), Some(val(1)))],
        )
        .unwrap();
        let mut d: Vec<String> = diff(&s, r1, r2).unwrap().into_iter().map(|x| x.0).collect();
        d.sort();
        assert_eq!(d, vec!["k4000", "k7", "new"]);
        assert_eq!(get(&s, r2, "k7").unwrap(), Some(val(99_999)));
        assert_eq!(get(&s, r2, "k4000").unwrap(), None);
    }
}
