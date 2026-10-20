//! A working copy: one device's view of one project, tied to one app through its connector.
//! save / history / restore / integrate. Restore and integrate always save the current state
//! first, so no operation can destroy the last good copy.

use crate::connector::Connector;
use crate::identity::{DeviceCert, DeviceKey};
use crate::merge::{self, Conflict, CONFLICT_PREFIX};
use crate::project::{self, Item, Project};
use crate::store::{Id, ObjectStore};
use crate::tree;
use anyhow::{bail, Result};
use std::collections::BTreeMap;

pub struct Repo<'a> {
    pub store: &'a dyn ObjectStore,
    pub project: Project,
    pub head: Option<Id>,
    pub cert: DeviceCert,
    pub key: DeviceKey,
}

pub enum Integrated {
    AlreadyHad,
    FastForward { applied: usize },
    Merged { version: Id, applied: usize, conflicts: Vec<Conflict> },
}

impl<'a> Repo<'a> {
    pub fn head_tree(&self) -> Result<Option<Id>> {
        match self.head {
            Some(h) => project::load_version(self.store, &h)?.tree(),
            None => Ok(None),
        }
    }

    /// Save the app's current state as a new version. `None` when nothing changed.
    pub fn save(&mut self, conn: &mut dyn Connector, message: &str) -> Result<Option<Id>> {
        self.save_seen(conn, message).map(|(v, _)| v)
    }

    /// `save`, also handing back the snapshot it saved. Restore and integrate build their changes
    /// on exactly that snapshot: reading the app a second time would see edits the user made in
    /// between, and applying the target over them would undo them unsaved (S6: 10 of Member1's
    /// newest tiles reverted in both windows with 0 clashes). An app that checks its own revision
    /// (Photo's `kopano.import`) then refuses the update instead, and the caller tries again.
    fn save_seen(&mut self, conn: &mut dyn Connector, message: &str) -> Result<(Option<Id>, BTreeMap<String, Item>)> {
        let info = conn.info();
        let snap = conn.snapshot()?;
        let head_tree = self.head_tree()?;
        let current = tree::entries(self.store, head_tree)?;
        let mut changes = Vec::new();
        for (key, item) in &snap {
            if !info.owns(&item.kind) {
                bail!("connector {} returned an item of kind {} it does not own", info.app, item.kind);
            }
            let id = item.put(self.store)?;
            if current.get(key) != Some(&id) {
                changes.push((key.clone(), Some(id)));
            }
        }
        // Items of this app that are gone from the app were removed. Other apps' items and
        // conflict records are left alone.
        for (key, id) in &current {
            if snap.contains_key(key) || key.starts_with(CONFLICT_PREFIX) {
                continue;
            }
            if info.owns(&Item::get(self.store, id)?.kind) {
                changes.push((key.clone(), None));
            }
        }
        if changes.is_empty() {
            return Ok((None, snap));
        }
        let root = tree::update(self.store, head_tree, changes)?;
        let parents = self.head.into_iter().collect();
        let v = project::commit(self.store, &self.project, parents, root, message, &self.cert, &self.key)?;
        self.store.flush()?;
        self.head = Some(v);
        Ok((Some(v), snap))
    }

    /// Save only what changed (E23): the app's change feed instead of a full snapshot, so a save
    /// costs the same at 1,000 or 1,000,000 items. `None` removes the item.
    pub fn save_changes(&mut self, info: &crate::connector::ConnectorInfo, changed: Vec<(String, Option<crate::project::Item>)>, message: &str) -> Result<Option<Id>> {
        let head_tree = self.head_tree()?;
        let mut changes = Vec::new();
        for (key, item) in changed {
            let now = tree::get(self.store, head_tree, &key)?;
            match item {
                Some(it) => {
                    if !info.owns(&it.kind) {
                        bail!("connector {} returned an item of kind {} it does not own", info.app, it.kind);
                    }
                    let id = it.put(self.store)?;
                    if now != Some(id) {
                        changes.push((key, Some(id)));
                    }
                }
                None => {
                    if now.is_some() {
                        changes.push((key, None));
                    }
                }
            }
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let root = tree::update(self.store, head_tree, changes)?;
        let parents = self.head.into_iter().collect();
        let v = project::commit(self.store, &self.project, parents, root, message, &self.cert, &self.key)?;
        self.store.flush()?;
        self.head = Some(v);
        Ok(Some(v))
    }

    /// Make the app match the tree of `target`; returns how many items were applied. `have` is
    /// the snapshot the caller just saved (`save_seen`), never a fresh read.
    fn apply_tree(&self, conn: &mut dyn Connector, target_tree: Option<Id>, have: &BTreeMap<String, Item>) -> Result<usize> {
        let info = conn.info();
        let want = tree::entries(self.store, target_tree)?;
        let mut n = 0;
        for (key, id) in &want {
            let item = Item::get(self.store, id)?;
            if !info.owns(&item.kind) {
                continue;
            }
            if have.get(key) != Some(&item) {
                conn.apply(key, Some(&item))?;
                n += 1;
            }
        }
        for key in have.keys() {
            if !want.contains_key(key) {
                conn.apply(key, None)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Go back to an earlier version. Saves first, then records the restore as a new version,
    /// so the state before the restore stays in history.
    pub fn restore(&mut self, conn: &mut dyn Connector, version: &Id) -> Result<Option<Id>> {
        let (_, have) = self.save_seen(conn, "Before restore")?;
        let target = project::load_version(self.store, version)?;
        self.apply_tree(conn, target.tree()?, &have)?;
        self.save(conn, &format!("Restored to {}", version.short()))
    }

    /// Bring in someone else's verified head.
    pub fn integrate(&mut self, conn: &mut dyn Connector, theirs: &Id) -> Result<Integrated> {
        project::verify_version(self.store, &self.project, theirs)?;
        self.integrate_checked(conn, theirs)
    }

    /// `integrate` for a head the caller has already verified under its own rules (v2 checks
    /// against the live members log, which can differ from the genesis members).
    pub fn integrate_checked(&mut self, conn: &mut dyn Connector, theirs: &Id) -> Result<Integrated> {
        let (_, have) = self.save_seen(conn, "Before bringing in changes")?;
        let Some(mine) = self.head else {
            let t = project::load_version(self.store, theirs)?;
            let applied = self.apply_tree(conn, t.tree()?, &have)?;
            self.head = Some(*theirs);
            return Ok(Integrated::FastForward { applied });
        };
        if mine == *theirs || project::is_ancestor(self.store, theirs, &mine)? {
            return Ok(Integrated::AlreadyHad);
        }
        if project::is_ancestor(self.store, &mine, theirs)? {
            let t = project::load_version(self.store, theirs)?;
            let applied = self.apply_tree(conn, t.tree()?, &have)?;
            self.head = Some(*theirs);
            return Ok(Integrated::FastForward { applied });
        }
        let info = conn.info();
        let policy = |k: &str| info.mergeable(k);
        // after a criss-cross there is more than one best common ancestor: keep the merge with
        // the fewest clashes (project::merge_bases)
        let mut bases: Vec<Option<Id>> = project::merge_bases(self.store, &mine, theirs, 4)?.into_iter().map(Some).collect();
        if bases.is_empty() {
            bases.push(None);
        }
        let mut best: Option<merge::MergeOutcome> = None;
        for b in bases {
            let base_tree = match b {
                Some(b) => project::load_version(self.store, &b)?.tree()?,
                None => None,
            };
            let out = merge::merge_trees(self.store, base_tree, self.head_tree()?, project::load_version(self.store, theirs)?.tree()?, &policy)?;
            if best.as_ref().map_or(true, |o| out.conflicts.len() < o.conflicts.len()) {
                best = Some(out);
            }
        }
        let out = best.expect("at least one base");
        let applied = self.apply_tree(conn, out.root, &have)?;
        let msg = if out.conflicts.is_empty() {
            "Merged".to_string()
        } else {
            format!("Merged with {} clash(es) kept for review", out.conflicts.len())
        };
        let v = project::commit(self.store, &self.project, vec![mine, *theirs], out.root, &msg, &self.cert, &self.key)?;
        self.store.flush()?;
        self.head = Some(v);
        Ok(Integrated::Merged { version: v, applied, conflicts: out.conflicts })
    }

    /// Open conflict records in the current version.
    pub fn conflicts(&self) -> Result<BTreeMap<String, Item>> {
        let mut out = BTreeMap::new();
        for (k, id) in tree::entries(self.store, self.head_tree()?)? {
            if k.starts_with(CONFLICT_PREFIX) {
                out.insert(k, Item::get(self.store, &id)?);
            }
        }
        Ok(out)
    }
}

/// Copy every object a version needs (history, trees, items) that `to` lacks. This is what a
/// sync transport moves; returns (objects, bytes) copied. Media blobs are NOT included: items
/// only reference them by hash, and media moves separately, on demand.
pub fn copy_version_closure(from: &dyn ObjectStore, to: &dyn ObjectStore, head: &Id) -> Result<(u64, u64)> {
    // Invariant: a version object is stored only after everything it needs (its parents' closures
    // and its own tree). A copy cut off part way then never leaves a version that looks present
    // but cannot be read, which would make every later copy skip it (found by E17: 53 of 100 runs stuck).
    let (mut n, mut bytes) = (0u64, 0u64);
    let mut order = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![(*head, false)];
    while let Some((v, expanded)) = stack.pop() {
        if expanded {
            order.push(v);
            continue;
        }
        if to.has(&v) || !seen.insert(v) {
            continue;
        }
        stack.push((v, true));
        for p in project::load_version(from, &v)?.parents()? {
            stack.push((p, false));
        }
    }
    for v in order {
        let ver = project::load_version(from, &v)?;
        let mut ids = Vec::new();
        tree::reachable(from, ver.tree()?, &mut ids)?;
        ids.push(v);
        for id in ids {
            if !to.has(&id) {
                let b = from.get(&id)?;
                to.put_verified(&id, &b)?;
                n += 1;
                bytes += b.len() as u64;
            }
        }
    }
    Ok((n, bytes))
}

#[cfg(test)]
mod criss_cross {
    use super::*;
    use crate::connector::{ConnectorInfo, KindInfo, CONTRACT_VERSION};
    use crate::identity::{DeviceKey, Provider};
    use crate::project::{Member, Role};
    use crate::store::MemStore;
    use serde_json::json;

    /// Items that can only be taken whole (like a Photo tile).
    struct Tiles(BTreeMap<String, Item>);
    impl Connector for Tiles {
        fn info(&self) -> ConnectorInfo {
            ConnectorInfo { app: "t".into(), contract: CONTRACT_VERSION, kinds: vec![KindInfo { kind: "t.tile".into(), lane: "l".into(), schema: 1, mergeable: false }] }
        }
        fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
            Ok(self.0.clone())
        }
        fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
            match item {
                Some(i) => self.0.insert(key.into(), i.clone()),
                None => self.0.remove(key),
            };
            Ok(())
        }
    }
    fn tile(v: &str) -> Item {
        Item { kind: "t.tile".into(), lane: "l".into(), schema: 1, state: json!({"hash": v}), assets: vec![] }
    }

    /// Both people merge each other's work at the same moment, then one paints again. Bringing
    /// that in must not report the shared work as a clash (S6: 10 false tile clashes).
    #[test]
    fn a_criss_cross_merge_does_not_clash_on_shared_work() -> Result<()> {
        let store = MemStore::default();
        let provider = Provider::generate();
        let (ka, kb) = (DeviceKey::generate(), DeviceKey::generate());
        let ca = provider.issue("alice", "Alice", &ka, "t", 3_600_000)?;
        let cb = provider.issue("bob", "Bob", &kb, "t", 3_600_000)?;
        let members = BTreeMap::from([("bob".to_string(), Member { role: Role::Editor, lanes: vec![] })]);
        let p = Project::create(&store, "x", vec![provider.public_hex()], &ca, &ka, members)?;
        let start = BTreeMap::from([("t".to_string(), tile("a")), ("x".to_string(), tile("a"))]);
        let mut a = Tiles(start.clone());
        let mut ra = Repo { store: &store, project: Project::load(&store, &p.id)?, head: None, cert: ca, key: ka };
        let v0 = ra.save(&mut a, "start")?.unwrap();
        let mut b = Tiles(start);
        let mut rb = Repo { store: &store, project: Project::load(&store, &p.id)?, head: Some(v0), cert: cb, key: kb };
        a.0.insert("x".into(), tile("x1"));
        let a1 = ra.save(&mut a, "alice paints x")?.unwrap();
        b.0.insert("t".into(), tile("b1"));
        let b1 = rb.save(&mut b, "bob paints t")?.unwrap();
        // at the same moment: Alice brings in B1; Bob paints t again and brings in A1
        assert!(matches!(ra.integrate(&mut a, &b1)?, Integrated::Merged { .. }));
        b.0.insert("t".into(), tile("b2"));
        let mb = match rb.integrate(&mut b, &a1)? {
            Integrated::Merged { version, .. } => version,
            _ => panic!("expected a merge"),
        };
        match ra.integrate(&mut a, &mb)? {
            Integrated::Merged { conflicts, .. } => assert!(conflicts.is_empty(), "false clashes: {:?}", conflicts.iter().map(|c| &c.key).collect::<Vec<_>>()),
            Integrated::FastForward { .. } => {}
            Integrated::AlreadyHad => panic!("Bob's second stroke was not brought in"),
        }
        assert_eq!(a.0["t"], tile("b2"));
        assert_eq!(a.0["x"], tile("x1"));
        Ok(())
    }

    /// A viewer may not edit, but it still merges what it receives. When the two heads clash on
    /// a tile, its merge records the clash. Everyone must accept that merge, because anyone can
    /// recompute it; refusing it left the viewer on its own content forever (E17 I1: 9 of 200
    /// seeds). A "merge" that is not the mechanical one stays refused.
    #[test]
    fn a_viewers_merge_that_records_a_clash_is_accepted() -> Result<()> {
        let store = MemStore::default();
        let provider = Provider::generate();
        let (ka, kc, kv) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
        let ca = provider.issue("alice", "Alice", &ka, "t", 3_600_000)?;
        let cc = provider.issue("carol", "Carol", &kc, "t", 3_600_000)?;
        let cv = provider.issue("vic", "Vic", &kv, "t", 3_600_000)?;
        let members = BTreeMap::from([
            ("carol".to_string(), Member { role: Role::Editor, lanes: vec![] }),
            ("vic".to_string(), Member { role: Role::Viewer, lanes: vec![] }),
        ]);
        let p = Project::create(&store, "x", vec![provider.public_hex()], &ca, &ka, members)?;
        let start = BTreeMap::from([("t".to_string(), tile("a"))]);
        let mut a = Tiles(start.clone());
        let mut ra = Repo { store: &store, project: Project::load(&store, &p.id)?, head: None, cert: ca.clone(), key: DeviceKey::from_bytes(ka.to_bytes()) };
        let v0 = ra.save(&mut a, "start")?.unwrap();
        let mut c = Tiles(start.clone());
        let mut rc = Repo { store: &store, project: Project::load(&store, &p.id)?, head: Some(v0), cert: cc, key: kc };
        a.0.insert("t".into(), tile("a1"));
        let a1 = ra.save(&mut a, "alice paints t")?.unwrap();
        c.0.insert("t".into(), tile("c1"));
        let c1 = rc.save(&mut c, "carol paints t")?.unwrap();
        let mut v = Tiles(start);
        let mut rv = Repo { store: &store, project: Project::load(&store, &p.id)?, head: Some(v0), cert: cv.clone(), key: DeviceKey::from_bytes(kv.to_bytes()) };
        assert!(matches!(rv.integrate(&mut v, &a1)?, Integrated::FastForward { .. }));
        let merged = match rv.integrate(&mut v, &c1)? {
            Integrated::Merged { version, conflicts, .. } => {
                assert_eq!(conflicts.len(), 1, "the tile clash is recorded");
                version
            }
            _ => panic!("expected a merge"),
        };
        let project = Project::load(&store, &p.id)?;
        project::verify_version(&store, &project, &merged)?;
        // Alice takes it and lands on the same content as Vic.
        assert!(matches!(ra.integrate(&mut a, &merged)?, Integrated::FastForward { .. }));
        assert_eq!(ra.head_tree()?, rv.head_tree()?);

        // Not mechanical: Vic paints the tile himself inside a merge. Refused.
        let a1_tree = project::load_version(&store, &a1)?.tree()?;
        let forged = tree::update(&store, a1_tree, vec![("t".into(), Some(tile("v").put(&store)?))])?;
        let f = project::commit(&store, &project, vec![a1, c1], forged, "forged", &cv, &kv)?;
        assert!(project::verify_version(&store, &project, &f).is_err(), "a viewer's own value inside a merge");
        // Not mechanical: a clash record the merge would not write. Refused.
        let fake = Item { kind: "kopano.conflict".into(), lane: crate::merge::CONFLICT_LANE.into(), schema: 1, state: json!({"key": "t", "fields": ["made up"], "mine": null, "theirs": null}), assets: vec![] };
        let forged = tree::update(&store, a1_tree, vec![(format!("{CONFLICT_PREFIX}t/x-y"), Some(fake.put(&store)?))])?;
        let f = project::commit(&store, &project, vec![a1, c1], forged, "forged record", &cv, &kv)?;
        assert!(project::verify_version(&store, &project, &f).is_err(), "a made-up clash record");
        Ok(())
    }
}
