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
            return Ok(None);
        }
        let root = tree::update(self.store, head_tree, changes)?;
        let parents = self.head.into_iter().collect();
        let v = project::commit(self.store, &self.project, parents, root, message, &self.cert, &self.key)?;
        self.store.flush()?;
        self.head = Some(v);
        Ok(Some(v))
    }

    /// Make the app match the tree of `target`; returns how many items were applied.
    fn apply_tree(&self, conn: &mut dyn Connector, target_tree: Option<Id>) -> Result<usize> {
        let info = conn.info();
        let want = tree::entries(self.store, target_tree)?;
        let have = conn.snapshot()?;
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
        self.save(conn, "Before restore")?;
        let target = project::load_version(self.store, version)?;
        self.apply_tree(conn, target.tree()?)?;
        self.save(conn, &format!("Restored to {}", version.short()))
    }

    /// Bring in someone else's verified head.
    pub fn integrate(&mut self, conn: &mut dyn Connector, theirs: &Id) -> Result<Integrated> {
        project::verify_version(self.store, &self.project, theirs)?;
        self.save(conn, "Before bringing in changes")?;
        let Some(mine) = self.head else {
            let t = project::load_version(self.store, theirs)?;
            let applied = self.apply_tree(conn, t.tree()?)?;
            self.head = Some(*theirs);
            return Ok(Integrated::FastForward { applied });
        };
        if mine == *theirs || project::is_ancestor(self.store, theirs, &mine)? {
            return Ok(Integrated::AlreadyHad);
        }
        if project::is_ancestor(self.store, &mine, theirs)? {
            let t = project::load_version(self.store, theirs)?;
            let applied = self.apply_tree(conn, t.tree()?)?;
            self.head = Some(*theirs);
            return Ok(Integrated::FastForward { applied });
        }
        let base = project::common_ancestor(self.store, &mine, theirs)?;
        let base_tree = match base {
            Some(b) => project::load_version(self.store, &b)?.tree()?,
            None => None,
        };
        let info = conn.info();
        let policy = |k: &str| info.mergeable(k);
        let out = merge::merge_trees(
            self.store,
            base_tree,
            self.head_tree()?,
            project::load_version(self.store, theirs)?.tree()?,
            &policy,
        )?;
        let applied = self.apply_tree(conn, out.root)?;
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
    let (mut n, mut bytes) = (0u64, 0u64);
    let mut stack = vec![*head];
    while let Some(v) = stack.pop() {
        if to.has(&v) {
            continue;
        }
        let ver = project::load_version(from, &v)?;
        let mut ids = Vec::new();
        tree::reachable(from, ver.tree()?, &mut ids)?;
        for id in ids {
            if !to.has(&id) {
                let b = from.get(&id)?;
                to.put_verified(&id, &b)?;
                n += 1;
                bytes += b.len() as u64;
            }
        }
        stack.extend(ver.parents()?);
        let vb = from.get(&v)?;
        to.put_verified(&v, &vb)?;
        n += 1;
        bytes += vb.len() as u64;
    }
    Ok((n, bytes))
}
