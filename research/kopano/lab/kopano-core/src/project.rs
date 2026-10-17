//! Projects, items, versions and permissions. Nothing here knows any application.
//!
//! - A **project** starts from a signed genesis record (its id is the genesis object's hash). The
//!   genesis names the trusted identity providers and the members with their roles and lanes.
//! - An **item** is one thing an app manages (a photo's edit, a clip, a layer, a note). Kopano sees
//!   only: its kind, its lane, a schema number, a JSON state, and the media it uses (by hash).
//! - A **version** is a signed save: parents, the manifest root, what lanes it touched, a message,
//!   and the author's device certificate inline so anyone can check it offline.

use crate::identity::{now_ms, DeviceCert, DeviceKey, Signed};
use crate::store::{Id, ObjectStore};
use crate::tree;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
    Admin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Member {
    pub role: Role,
    /// Lanes this member may write. Empty with role Editor = all lanes. Commenters may write the
    /// "notes" lane only.
    #[serde(default)]
    pub lanes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Genesis {
    pub kind: String,
    pub name: String,
    pub created_ms: u64,
    pub providers: Vec<String>,
    pub owner: String,
    pub members: BTreeMap<String, Member>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignedGenesis {
    pub genesis: Signed<Genesis>,
    pub cert: DeviceCert,
}

pub struct Project {
    pub id: Id,
    pub genesis: Genesis,
}

impl Project {
    pub fn create(store: &dyn ObjectStore, name: &str, providers: Vec<String>, owner_cert: &DeviceCert, owner_key: &DeviceKey, members: BTreeMap<String, Member>) -> Result<Self> {
        let mut members = members;
        members.insert(owner_cert.body.account.clone(), Member { role: Role::Admin, lanes: vec![] });
        let g = Genesis { kind: "kopano.project".into(), name: name.into(), created_ms: now_ms(), providers, owner: owner_cert.body.account.clone(), members };
        let sg = SignedGenesis { genesis: Signed::new(g, owner_key.signing())?, cert: owner_cert.clone() };
        let id = store.put(&serde_json::to_vec(&sg)?)?;
        store.flush()?;
        Self::load(store, &id)
    }

    /// Load and fully check a project genesis (signature, owner's certificate, owner is admin).
    pub fn load(store: &dyn ObjectStore, id: &Id) -> Result<Self> {
        let sg: SignedGenesis = serde_json::from_slice(&store.get(id)?).context("not a project")?;
        let signer = sg.genesis.verify()?;
        let g = sg.genesis.body.clone();
        sg.cert.check(&g.providers, sg.genesis.body.created_ms)?;
        if sg.cert.body.device != hex::encode(signer.to_bytes()) || sg.cert.body.account != g.owner {
            bail!("project genesis is not signed by its owner's device");
        }
        Ok(Self { id: *id, genesis: g })
    }

    pub fn may_write(&self, account: &str, lane: &str) -> bool {
        if lane == crate::merge::CONFLICT_LANE {
            // Recording a clash is open to anyone who may contribute at all.
            return self.genesis.members.get(account).is_some_and(|m| m.role >= Role::Commenter);
        }
        match self.genesis.members.get(account) {
            Some(Member { role: Role::Admin, .. }) => true,
            Some(Member { role: Role::Editor, lanes }) => lanes.is_empty() || lanes.iter().any(|l| l == lane),
            Some(Member { role: Role::Commenter, .. }) => lane == "notes",
            _ => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AssetRef {
    /// BLAKE3 of the file bytes: the same id iroh-blobs uses, so a ref is also a fetch address.
    pub hash: String,
    pub size: u64,
    /// original | proxy | preview | export: lets a peer fetch only the rung it needs.
    pub role: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: String,
    pub lane: String,
    pub schema: u32,
    pub state: Value,
    #[serde(default)]
    pub assets: Vec<AssetRef>,
}

impl Item {
    pub fn put(&self, store: &dyn ObjectStore) -> Result<Id> {
        store.put(&serde_json::to_vec(self)?)
    }
    pub fn get(store: &dyn ObjectStore, id: &Id) -> Result<Self> {
        serde_json::from_slice(&store.get(id)?).context("not an item")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionBody {
    pub project: String,
    pub parents: Vec<String>,
    /// Manifest root, or None for an empty project.
    pub tree: Option<String>,
    pub lanes: Vec<String>,
    pub time_ms: u64,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Version {
    pub v: Signed<VersionBody>,
    pub cert: DeviceCert,
}

impl Version {
    pub fn tree(&self) -> Result<Option<Id>> {
        self.v.body.tree.as_deref().map(Id::parse).transpose()
    }
    pub fn parents(&self) -> Result<Vec<Id>> {
        self.v.body.parents.iter().map(|p| Id::parse(p)).collect()
    }
    pub fn author(&self) -> &str {
        &self.cert.body.name
    }
}

pub fn load_version(store: &dyn ObjectStore, id: &Id) -> Result<Version> {
    serde_json::from_slice(&store.get(id)?).with_context(|| format!("{} is not a version", id.short()))
}

/// Create and store a signed version. Lanes touched are computed from the diff with the first
/// parent, so a version can never claim fewer lanes than it really changed.
pub fn commit(store: &dyn ObjectStore, project: &Project, parents: Vec<Id>, tree_root: Option<Id>, message: &str, cert: &DeviceCert, key: &DeviceKey) -> Result<Id> {
    let lanes = touched_lanes(store, parents.first().copied(), tree_root)?;
    let body = VersionBody {
        project: project.id.hex(),
        parents: parents.iter().map(Id::hex).collect(),
        tree: tree_root.map(|r| r.hex()),
        lanes: lanes.into_iter().collect(),
        time_ms: now_ms(),
        message: message.into(),
    };
    let v = Version { v: Signed::new(body, key.signing())?, cert: cert.clone() };
    store.put(&serde_json::to_vec(&v)?)
}

fn touched_lanes(store: &dyn ObjectStore, parent: Option<Id>, root: Option<Id>) -> Result<BTreeSet<String>> {
    let parent_tree = match parent {
        Some(p) => load_version(store, &p)?.tree()?,
        None => None,
    };
    let mut lanes = BTreeSet::new();
    for (_, a, b) in tree::diff(store, parent_tree, root)? {
        for id in [a, b].into_iter().flatten() {
            lanes.insert(Item::get(store, &id)?.lane);
        }
    }
    Ok(lanes)
}

/// Full acceptance check for a version received from anyone: signature, certificate, membership,
/// lane permission (recomputed from the data, not trusted from the claim), project match.
/// Requires the version's objects and its first parent to be present.
pub fn verify_version(store: &dyn ObjectStore, project: &Project, id: &Id) -> Result<Version> {
    verify_version_by(store, project, id, &|account: &str, lane: &str| project.may_write(account, lane))
}

/// `verify_version` with the permission rule supplied (v2 uses the live members log).
pub fn verify_version_by(store: &dyn ObjectStore, project: &Project, id: &Id, may_write: &dyn Fn(&str, &str) -> bool) -> Result<Version> {
    let v = load_version(store, id)?;
    let signer = v.v.verify()?;
    v.cert.check(&project.genesis.providers, v.v.body.time_ms)?;
    if v.cert.body.device != hex::encode(signer.to_bytes()) {
        bail!("version {} was signed by a device its certificate does not name", id.short());
    }
    if v.v.body.project != project.id.hex() {
        bail!("version {} belongs to another project", id.short());
    }
    let parents = v.parents()?;
    let root = v.tree()?;
    let first_tree = match parents.first() {
        Some(p) => load_version(store, p)?.tree()?,
        None => None,
    };
    // Other parents' trees: a merge may carry their changes in unchanged, because those parents
    // are verified versions themselves (sync verifies parents before children).
    let mut other_trees = Vec::new();
    for p in parents.iter().skip(1) {
        other_trees.push(load_version(store, p)?.tree()?);
    }
    let account = &v.cert.body.account;
    for (key, before, after) in tree::diff(store, first_tree, root)? {
        let mut from_other_parent = false;
        for t in &other_trees {
            if tree::get(store, *t, &key)? == after {
                from_other_parent = true;
            }
        }
        if from_other_parent {
            continue;
        }
        // A new value: the author needs write permission on the lane of what changed.
        for item in [before, after].into_iter().flatten() {
            let lane = Item::get(store, &item)?.lane;
            if !may_write(account, &lane) {
                bail!("{} may not change the {lane} lane (version {})", v.cert.body.name, id.short());
            }
        }
    }
    Ok(v)
}

/// All versions reachable from `heads`, newest first by time.
pub fn history(store: &dyn ObjectStore, heads: &[Id]) -> Result<Vec<(Id, Version)>> {
    let mut seen = BTreeSet::new();
    let mut q: VecDeque<Id> = heads.iter().copied().collect();
    let mut out = Vec::new();
    while let Some(id) = q.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        let v = load_version(store, &id)?;
        q.extend(v.parents()?);
        out.push((id, v));
    }
    out.sort_by(|a, b| b.1.v.body.time_ms.cmp(&a.1.v.body.time_ms));
    Ok(out)
}

pub fn is_ancestor(store: &dyn ObjectStore, maybe_ancestor: &Id, of: &Id) -> Result<bool> {
    Ok(history(store, &[*of])?.iter().any(|(id, _)| id == maybe_ancestor))
}

/// Nearest common ancestor of two versions (breadth first from `b`).
pub fn common_ancestor(store: &dyn ObjectStore, a: &Id, b: &Id) -> Result<Option<Id>> {
    let anc: BTreeSet<Id> = history(store, &[*a])?.into_iter().map(|(id, _)| id).collect();
    let mut seen = BTreeSet::new();
    let mut q = VecDeque::from([*b]);
    while let Some(id) = q.pop_front() {
        if anc.contains(&id) {
            return Ok(Some(id));
        }
        if seen.insert(id) {
            q.extend(load_version(store, &id)?.parents()?);
        }
    }
    Ok(None)
}
