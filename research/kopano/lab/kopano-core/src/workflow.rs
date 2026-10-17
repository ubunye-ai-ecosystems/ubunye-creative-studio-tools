//! Kopano v2 workflow (the owner's model, 2026-10-18): one official line, `main`, owned by the
//! person who created the project; everyone else works on their own branch and their work reaches
//! main only when an approver accepts it. The owner can make others approvers, add and remove
//! people and block a lost device, all as signed entries in a members log.
//!
//! Why: with one shared line that everyone merges into, two people merging at once can disagree
//! forever (E17: 38 of 200 simulated teams never converged). Here every branch has one writer and
//! main moves only by approval, so everyone simply follows main.
//!
//! Objects (content-addressed like everything else, so they sync with the same machinery):
//! - members entry: {op, previous entries it saw}, signed by an admin
//! - ref update: "ref <name> now points at version V", signed, citing the members it was made
//!   under and the update it replaces
//!
//! Rules a receiver checks:
//! - `branch/<account>`: only that account's devices, while the account is a current member and
//!   the device is not blocked.
//! - `main`: only an approver according to the members the update cites (so later demotions do
//!   not invalidate earlier approvals). Known limit: an approver who was demoted but never saw the
//!   demotion can still approve until they sync; that is inherent to working offline and is shown
//!   to the owner as "approved by a former approver" for review.
//! - Two updates of the same ref replacing the same predecessor (two approvers at once, or one
//!   person's two devices): the larger (seq, id) is the tip everyone follows; the other version
//!   stays reachable and is offered for approval again, so nothing is lost.

use crate::identity::{now_ms, DeviceCert, DeviceKey, Signed};
use crate::merge::{self, CONFLICT_PREFIX};
use crate::project::{self, Member, Project, Role};
use crate::store::{Id, ObjectStore};
use crate::tree;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAIN: &str = "main";

pub fn branch_of(account: &str) -> String {
    format!("branch/{account}")
}

// ---------------------------------------------------------------- members log

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MemberOp {
    /// Add someone or change their role, lanes or approver right.
    Set { account: String, role: Role, lanes: Vec<String>, approver: bool },
    Remove { account: String },
    /// Block one device (a lost or stolen laptop); the person stays a member.
    BlockDevice { device: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemberEntry {
    pub kind: String,
    pub project: String,
    /// The latest entries the signer had seen (usually one).
    pub after: Vec<String>,
    pub op: MemberOp,
    pub time_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignedMemberEntry {
    pub entry: Signed<MemberEntry>,
    pub cert: DeviceCert,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemberInfo {
    pub role: Role,
    pub lanes: Vec<String>,
    pub approver: bool,
}

impl Default for Role {
    fn default() -> Self {
        Role::Viewer
    }
}

#[derive(Clone, Debug, Default)]
pub struct Members {
    pub owner: String,
    pub map: BTreeMap<String, MemberInfo>,
    pub blocked: BTreeSet<String>,
}

impl Members {
    pub fn from_genesis(project: &Project) -> Members {
        let mut map = BTreeMap::new();
        for (acct, Member { role, lanes }) in &project.genesis.members {
            map.insert(acct.clone(), MemberInfo { role: *role, lanes: lanes.clone(), approver: *role == Role::Admin });
        }
        Members { owner: project.genesis.owner.clone(), map, blocked: BTreeSet::new() }
    }
    pub fn is_member(&self, account: &str) -> bool {
        account == self.owner || self.map.contains_key(account)
    }
    pub fn is_admin(&self, account: &str) -> bool {
        account == self.owner || self.map.get(account).is_some_and(|m| m.role == Role::Admin)
    }
    pub fn is_approver(&self, account: &str) -> bool {
        self.is_admin(account) || self.map.get(account).is_some_and(|m| m.approver)
    }
    pub fn device_ok(&self, cert: &DeviceCert) -> bool {
        self.is_member(&cert.body.account) && !self.blocked.contains(&cert.body.device)
    }
    pub fn may_write(&self, account: &str, lane: &str) -> bool {
        if account == self.owner {
            return true;
        }
        let Some(m) = self.map.get(account) else { return false };
        if lane == merge::CONFLICT_LANE {
            return m.role >= Role::Commenter;
        }
        match m.role {
            Role::Admin => true,
            Role::Editor => m.lanes.is_empty() || m.lanes.iter().any(|l| l == lane),
            Role::Commenter => lane == "notes",
            Role::Viewer => false,
        }
    }
    fn apply(&mut self, op: &MemberOp) {
        match op {
            MemberOp::Set { account, role, lanes, approver } => {
                if *account != self.owner {
                    self.map.insert(account.clone(), MemberInfo { role: *role, lanes: lanes.clone(), approver: *approver });
                }
            }
            MemberOp::Remove { account } => {
                if *account != self.owner {
                    self.map.remove(account);
                }
            }
            MemberOp::BlockDevice { device } => {
                self.blocked.insert(device.clone());
            }
        }
    }
}

pub fn add_member_entry(store: &dyn ObjectStore, project: &Project, after: Vec<Id>, op: MemberOp, cert: &DeviceCert, key: &DeviceKey) -> Result<Id> {
    let e = MemberEntry { kind: "kopano.members".into(), project: project.id.hex(), after: after.iter().map(Id::hex).collect(), op, time_ms: now_ms() };
    let s = SignedMemberEntry { entry: Signed::new(e, key.signing())?, cert: cert.clone() };
    store.put(&serde_json::to_vec(&s)?)
}

fn load_member_entry(store: &dyn ObjectStore, id: &Id) -> Result<SignedMemberEntry> {
    serde_json::from_slice(&store.get(id)?).context("not a members entry")
}

/// Members after applying `known` entries (and nothing else) in a deterministic order: an entry
/// is applied once everything it saw is applied, ties broken by id. Entries signed by someone who
/// is not an admin at that point, or by a blocked device, are ignored.
pub fn members(store: &dyn ObjectStore, project: &Project, known: &BTreeSet<Id>) -> Result<Members> {
    let mut m = Members::from_genesis(project);
    let mut entries: BTreeMap<Id, SignedMemberEntry> = BTreeMap::new();
    for id in known {
        if let Ok(e) = load_member_entry(store, id) {
            if e.entry.body.project == project.id.hex() {
                entries.insert(*id, e);
            }
        }
    }
    let mut done: BTreeSet<Id> = BTreeSet::new();
    loop {
        let ready: Option<Id> = entries
            .iter()
            .filter(|(id, _)| !done.contains(*id))
            .filter(|(_, e)| e.entry.body.after.iter().all(|a| Id::parse(a).map(|a| done.contains(&a) || !entries.contains_key(&a)).unwrap_or(true)))
            .map(|(id, _)| *id)
            .next();
        let Some(id) = ready else { break };
        done.insert(id);
        let e = &entries[&id];
        let ok = e.entry.verify().is_ok()
            && e.cert.check(&project.genesis.providers, e.entry.body.time_ms).is_ok()
            && e.cert.body.device == e.entry.signer
            && m.is_admin(&e.cert.body.account)
            && !m.blocked.contains(&e.cert.body.device);
        if ok {
            m.apply(&e.entry.body.op);
        }
    }
    Ok(m)
}

/// Every entry reachable from `tips` through `after` (the members a signer had seen).
pub fn member_closure(store: &dyn ObjectStore, tips: &[Id]) -> BTreeSet<Id> {
    let mut out = BTreeSet::new();
    let mut stack: Vec<Id> = tips.to_vec();
    while let Some(id) = stack.pop() {
        if !out.insert(id) {
            continue;
        }
        if let Ok(e) = load_member_entry(store, &id) {
            stack.extend(e.entry.body.after.iter().filter_map(|a| Id::parse(a).ok()));
        }
    }
    out
}

/// Entries in `known` that no other known entry saw: the latest state to cite.
pub fn member_tips(store: &dyn ObjectStore, known: &BTreeSet<Id>) -> Vec<Id> {
    let mut seen = BTreeSet::new();
    for id in known {
        if let Ok(e) = load_member_entry(store, id) {
            seen.extend(e.entry.body.after.iter().filter_map(|a| Id::parse(a).ok()));
        }
    }
    known.iter().filter(|i| !seen.contains(*i)).copied().collect()
}

// ---------------------------------------------------------------- refs

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RefUpdate {
    pub kind: String,
    pub project: String,
    pub name: String,
    pub version: String,
    /// The update this one replaces (None for the first).
    pub previous: Option<String>,
    pub seq: u64,
    /// Members entries the signer had seen.
    pub members: Vec<String>,
    pub time_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignedRef {
    pub update: Signed<RefUpdate>,
    pub cert: DeviceCert,
}

pub fn load_ref(store: &dyn ObjectStore, id: &Id) -> Result<SignedRef> {
    serde_json::from_slice(&store.get(id)?).context("not a ref update")
}

#[allow(clippy::too_many_arguments)]
pub fn update_ref(store: &dyn ObjectStore, project: &Project, name: &str, version: &Id, previous: Option<(Id, u64)>, members_seen: &[Id], cert: &DeviceCert, key: &DeviceKey) -> Result<Id> {
    let u = RefUpdate {
        kind: "kopano.ref".into(),
        project: project.id.hex(),
        name: name.into(),
        version: version.hex(),
        previous: previous.map(|(p, _)| p.hex()),
        seq: previous.map(|(_, s)| s + 1).unwrap_or(1),
        members: members_seen.iter().map(Id::hex).collect(),
        time_ms: now_ms(),
    };
    let s = SignedRef { update: Signed::new(u, key.signing())?, cert: cert.clone() };
    store.put(&serde_json::to_vec(&s)?)
}

/// Check one ref update. `current`: the receiver's latest members (for branches).
pub fn check_ref(store: &dyn ObjectStore, project: &Project, current: &Members, id: &Id) -> Result<RefUpdate> {
    let s = load_ref(store, id)?;
    let u = &s.update.body;
    s.update.verify()?;
    s.cert.check(&project.genesis.providers, u.time_ms)?;
    if s.cert.body.device != s.update.signer {
        bail!("ref update signed by a device its certificate does not name");
    }
    if u.project != project.id.hex() {
        bail!("ref update for another project");
    }
    let acct = &s.cert.body.account;
    if u.name == MAIN {
        let cited: Vec<Id> = u.members.iter().filter_map(|m| Id::parse(m).ok()).collect();
        let at = members(store, project, &member_closure(store, &cited))?;
        if !at.is_approver(acct) || at.blocked.contains(&s.cert.body.device) {
            bail!("{} may not approve into main", s.cert.body.name);
        }
    } else if u.name == branch_of(acct) {
        if !current.device_ok(&s.cert) {
            bail!("{} is no longer on the project (or this device is blocked)", s.cert.body.name);
        }
    } else {
        bail!("{} may not move {}", s.cert.body.name, u.name);
    }
    Ok(u.clone())
}

/// The tip of every ref among `known` valid updates: an update nobody valid replaced; if several
/// (a fork), the larger (seq, id). Also returns the forked-off versions so they can be re-offered.
pub fn tips(store: &dyn ObjectStore, project: &Project, current: &Members, known: &BTreeSet<Id>) -> (BTreeMap<String, (Id, RefUpdate)>, Vec<Id>) {
    let mut valid: BTreeMap<Id, RefUpdate> = BTreeMap::new();
    for id in known {
        if let Ok(u) = check_ref(store, project, current, id) {
            valid.insert(*id, u);
        }
    }
    let replaced: BTreeSet<String> = valid.values().filter_map(|u| u.previous.clone()).collect();
    let mut out: BTreeMap<String, (Id, RefUpdate)> = BTreeMap::new();
    let mut forks = vec![];
    for (id, u) in &valid {
        if replaced.contains(&id.hex()) {
            continue;
        }
        match out.get(&u.name) {
            Some((bid, bu)) if (bu.seq, bid.hex()) >= (u.seq, id.hex()) => {
                if let Ok(v) = Id::parse(&u.version) {
                    forks.push(v);
                }
            }
            Some((_, bu)) => {
                if let Ok(v) = Id::parse(&bu.version) {
                    forks.push(v);
                }
                out.insert(u.name.clone(), (*id, u.clone()));
            }
            None => {
                out.insert(u.name.clone(), (*id, u.clone()));
            }
        }
    }
    (out, forks)
}

// ---------------------------------------------------------------- versions under v2 rules

/// Accept a version under v2: either it is vouched for by an approver (reachable from a valid main
/// update, checked by the caller), or its author may write every lane it newly changes according
/// to the receiver's current members.
pub fn verify_version_v2(store: &dyn ObjectStore, project: &Project, members: &Members, id: &Id) -> Result<project::Version> {
    project::verify_version_by(store, project, id, &|account: &str, lane: &str| members.may_write(account, lane))
}

/// An approver accepts `branch` into `main`: a merge version with both parents. Where both sides
/// changed the same setting, the approver's choice (`prefer_branch`) decides; clash records are
/// not written into main (the decision is the record, in the message).
pub fn approve(store: &dyn ObjectStore, project: &Project, main: &Id, branch: &Id, prefer_branch: bool, mergeable: &dyn merge::MergePolicy, cert: &DeviceCert, key: &DeviceKey) -> Result<(Id, usize)> {
    if project::is_ancestor(store, branch, main)? {
        bail!("nothing to approve: main already has this work");
    }
    let main_tree = project::load_version(store, main)?.tree()?;
    let branch_tree = project::load_version(store, branch)?.tree()?;
    let base = project::common_ancestor(store, main, branch)?;
    let base_tree = match base {
        Some(b) => project::load_version(store, &b)?.tree()?,
        None => None,
    };
    let out = merge::merge_trees(store, base_tree, main_tree, branch_tree, mergeable)?;
    let mut changes: Vec<tree::Change> = vec![];
    // drop clash records the merge produced; apply the approver's choice per clashing item
    for (k, _) in tree::entries(store, out.root)? {
        if k.starts_with(CONFLICT_PREFIX) && tree::get(store, main_tree, &k)?.is_none() {
            changes.push((k, None));
        }
    }
    for c in &out.conflicts {
        let pick = match (c.mine, c.theirs) {
            (Some(m), Some(t)) => {
                let (mi, ti) = (crate::project::Item::get(store, &m)?, crate::project::Item::get(store, &t)?);
                if mi.kind == ti.kind && mi.lane == ti.lane && mi.schema == ti.schema && mergeable.mergeable(&mi.kind) {
                    // setting by setting: only the clashing settings follow the approver's choice
                    let base_item = match base_tree {
                        Some(_) => tree::get(store, base_tree, &c.key)?.map(|i| crate::project::Item::get(store, &i)).transpose()?,
                        None => None,
                    };
                    let (state, _) = crate::jsonmerge::merge3_prefer(base_item.as_ref().map(|i| &i.state), &mi.state, &ti.state, prefer_branch);
                    let assets = if prefer_branch { ti.assets.clone() } else { mi.assets.clone() };
                    Some(crate::project::Item { state, assets, ..mi }.put(store)?)
                } else if prefer_branch {
                    Some(t)
                } else {
                    Some(m)
                }
            }
            (m, t) => {
                if prefer_branch {
                    t
                } else {
                    m
                }
            }
        };
        changes.push((c.key.clone(), pick));
    }
    let root = tree::update(store, out.root, changes)?;
    let n = out.conflicts.len();
    let msg = if n == 0 { "Approved".to_string() } else { format!("Approved, {n} clash(es) decided ({})", if prefer_branch { "theirs" } else { "main" }) };
    let v = project::commit(store, project, vec![*main, *branch], root, &msg, cert, key)?;
    store.flush()?;
    Ok((v, n))
}
