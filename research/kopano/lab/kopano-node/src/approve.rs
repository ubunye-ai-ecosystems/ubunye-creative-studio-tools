//! "I approve changes" projects (09 section 9): the owner's official line, `main`, moves only
//! when an approver approves; every teammate works on their own branch and sees their own edits
//! at once. The rules are kopano-core's workflow.rs (signed ref updates, members log, seal after a
//! members change, forks offered again); this file runs them inside kopanod.
//!
//! What each window shows (the project's `head` file on that device):
//! - the owner's: main. The owner's edits go straight to main.
//! - a member's: his branch, merged with the latest main; with nothing waiting, main itself.
//!
//! Sync: the same watch/push channel as live projects. A push carries the versions the watcher
//! lacks plus the ref updates and members entries it lacks; the receiver keeps them
//! (`refs.json`, `members.json`), checks every ref and the history under it (an approver's main:
//! signature and certificate; a branch: its author's lanes under the members the update cites)
//! and brings its window to what it should show.
//!
//! Live projects (no mode in the genesis) never come here.

use crate::{Node, Proj};
use anyhow::{bail, Context, Result};
use kopano_core::identity::now_ms;
use kopano_core::merge::CONFLICT_PREFIX;
use kopano_core::project::{self, Item};
use kopano_core::repo::Integrated;
use kopano_core::store::{Id, ObjectStore};
use kopano_core::tree;
use kopano_core::workflow::{self as wf, MemberOp, Members, RefUpdate, MAIN};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

pub type Tips = BTreeMap<String, (Id, RefUpdate)>;

/// This device's view of a project's refs and members (empty for live projects).
#[derive(Default)]
pub struct State {
    refs: Mutex<BTreeSet<Id>>,
    members: Mutex<BTreeSet<Id>>,
    /// versions whose history passed the checks (never checked twice)
    verified: Mutex<HashSet<Id>>,
    /// ref update -> passed `check_ref` + history, cleared when the members change
    checked: Mutex<HashMap<Id, bool>>,
    /// (refs, members) sizes -> tips: both sets only grow, so the sizes say when to look again
    tips: Mutex<Option<((usize, usize), Tips, Vec<Id>)>>,
    /// the owner looking at a member's version in their own window (saves paused meanwhile)
    pub viewing: Mutex<Option<Viewing>>,
    /// the last `review` event sent, so the same one is not sent again
    last: Mutex<String>,
}

#[derive(Clone, Debug)]
pub struct Viewing {
    pub account: String,
    pub version: Id,
    pub keys: Vec<String>,
    /// "member" (their version shows) or "main"
    pub side: String,
}

fn read_ids(f: &Path) -> BTreeSet<Id> {
    let list: Vec<String> = std::fs::read(f).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.iter().filter_map(|s| Id::parse(s).ok()).collect()
}

fn write_ids(f: &Path, ids: &BTreeSet<Id>) -> Result<()> {
    kopano_core::store::write_atomic(f, &serde_json::to_vec(&ids.iter().map(Id::hex).collect::<Vec<_>>())?)
}

impl State {
    pub fn load(dir: &Path) -> Self {
        State { refs: Mutex::new(read_ids(&dir.join("refs.json"))), members: Mutex::new(read_ids(&dir.join("members.json"))), ..Default::default() }
    }
    pub fn refs(&self) -> BTreeSet<Id> {
        self.refs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn members(&self) -> BTreeSet<Id> {
        self.members.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn viewing(&self) -> Option<Viewing> {
        self.viewing.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// An "I approve changes" project.
pub fn on(p: &Proj) -> bool {
    p.project.approve_mode()
}

/// The kinds merged setting by setting when an approver decides a clash. Photo tiles are taken
/// whole (as their connector declares); every other kind the apps have is JSON settings.
pub fn mergeable(kind: &str) -> bool {
    !kind.starts_with("photo.tile")
}

/// Keep ref updates and members entries that arrived (only objects that are here and are what
/// they claim to be, for this project). True when anything was new.
pub fn learn(p: &Proj, refs: &[Id], members: &[Id]) -> Result<bool> {
    let pid = p.project.id.hex();
    let ok_ref = |id: &Id| wf::load_ref(&p.store, id).is_ok_and(|r| r.update.body.project == pid);
    let ok_member = |id: &Id| p.store.get(id).ok().and_then(|b| serde_json::from_slice::<wf::SignedMemberEntry>(&b).ok()).is_some_and(|e| e.entry.body.project == pid);
    let mut new = false;
    {
        let mut set = p.review.refs.lock().unwrap_or_else(|e| e.into_inner());
        let before = set.len();
        let add: Vec<Id> = refs.iter().filter(|r| !set.contains(r) && ok_ref(r)).copied().collect();
        set.extend(add);
        if set.len() != before {
            write_ids(&p.dir.join("refs.json"), &set)?;
            new = true;
        }
    }
    {
        let mut set = p.review.members.lock().unwrap_or_else(|e| e.into_inner());
        let before = set.len();
        let add: Vec<Id> = members.iter().filter(|m| !set.contains(m) && ok_member(m)).copied().collect();
        set.extend(add);
        if set.len() != before {
            write_ids(&p.dir.join("members.json"), &set)?;
            p.review.checked.lock().unwrap_or_else(|e| e.into_inner()).clear();
            new = true;
        }
    }
    Ok(new)
}

/// Who is on the project now: the genesis plus the signed members log.
pub fn members(p: &Proj) -> Result<Members> {
    wf::members(&p.store, &p.project, &p.review.members())
}

/// Is this ref update one everyone may follow: valid by workflow.rs, and its version and the
/// whole history under it here and allowed (an approver's main vouches for what it carries; a
/// branch's versions must keep to their authors' lanes).
fn usable(p: &Proj, current: &Members, r: &Id) -> bool {
    if let Some(ok) = p.review.checked.lock().unwrap_or_else(|e| e.into_inner()).get(r) {
        return *ok;
    }
    let ok = (|| -> Result<bool> {
        let u = wf::check_ref(&p.store, &p.project, current, r)?;
        let v = Id::parse(&u.version)?;
        if p.review.verified.lock().unwrap_or_else(|e| e.into_inner()).contains(&v) {
            return Ok(true);
        }
        let Ok(hist) = project::history(&p.store, &[v]) else { return Ok(false) }; // not all here yet
        let s = wf::load_ref(&p.store, r)?;
        let cited: Vec<Id> = s.update.body.members.iter().filter_map(|m| Id::parse(m).ok()).collect();
        let at = wf::members(&p.store, &p.project, &wf::member_closure(&p.store, &cited))?;
        let verified = p.review.verified.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (vid, _) in hist.iter().rev() {
            if verified.contains(vid) {
                continue;
            }
            let res = if wf::approver_ref(&u.name) {
                project::verify_version_by(&p.store, &p.project, vid, &|_, _| true).map(|_| ())
            } else {
                wf::verify_version_v2(&p.store, &p.project, &at, vid).map(|_| ())
            };
            if let Err(e) = res {
                eprintln!("[kopanod] {}: {} refused: {e:#}", p.project.id.short(), u.name);
                return Ok(false);
            }
        }
        p.review.verified.lock().unwrap_or_else(|e| e.into_inner()).extend(hist.into_iter().map(|(id, _)| id));
        Ok(true)
    })()
    .unwrap_or(false);
    // a version still on its way is asked again next time; a refusal is final
    if ok || wf::load_ref(&p.store, r).ok().and_then(|s| Id::parse(&s.update.body.version).ok()).is_some_and(|v| project::history(&p.store, &[v]).is_ok()) {
        p.review.checked.lock().unwrap_or_else(|e| e.into_inner()).insert(*r, ok);
    }
    ok
}

/// The tip of every ref this device may follow, and the versions forked off (offered again).
pub fn tips(p: &Proj) -> Result<(Tips, Vec<Id>)> {
    let (refs, mem) = (p.review.refs(), p.review.members());
    let size = (refs.len(), mem.len());
    if let Some((s, t, f)) = p.review.tips.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if *s == size {
            return Ok((t.clone(), f.clone()));
        }
    }
    let current = wf::members(&p.store, &p.project, &mem)?;
    let ok: BTreeSet<Id> = refs.iter().filter(|r| usable(p, &current, r)).copied().collect();
    let (t, f) = wf::tips(&p.store, &p.project, &current, &ok);
    // a version still on its way: do not keep this answer
    let complete = ok.len() == refs.len() || refs.iter().all(|r| p.review.checked.lock().unwrap_or_else(|e| e.into_inner()).contains_key(r));
    if complete {
        *p.review.tips.lock().unwrap_or_else(|e| e.into_inner()) = Some((size, t.clone(), f.clone()));
    }
    Ok((t, f))
}

fn version_of(t: &Tips, name: &str) -> Option<Id> {
    t.get(name).and_then(|(_, u)| Id::parse(&u.version).ok())
}

fn is_anc(p: &Proj, a: &Id, b: &Id) -> bool {
    a == b || project::is_ancestor(&p.store, a, b).unwrap_or(false)
}

/// Still the declined work: since `declined`, the member's branch only took main in (catch-up
/// merges), no edit of his own. Main moving on must not offer declined work for review again.
fn still_declined(p: &Proj, tip: &Id, declined: &Id, main: &Id) -> bool {
    if tip == declined {
        return true;
    }
    if !is_anc(p, declined, tip) {
        return false;
    }
    let Ok(known) = project::history(&p.store, &[*main, *declined]) else { return false };
    let known: BTreeSet<Id> = known.into_iter().map(|(id, _)| id).collect();
    project::history(&p.store, &[*tip]).is_ok_and(|h| h.iter().filter(|(id, _)| !known.contains(id)).all(|(_, v)| v.parents().is_ok_and(|ps| ps.len() == 2)))
}

/// `v` brings nothing main does not have: it is in main, or merging it into main changes nothing
/// (S8: after two approvers approved at once, catch-up merges in a member's branch held only what
/// main got by another way, and were offered for review as "changed 0 photos").
fn adds_nothing(p: &Proj, v: &Id, main: &Id) -> bool {
    if is_anc(p, v, main) {
        return true;
    }
    (|| -> Result<bool> {
        let base = project::common_ancestor(&p.store, main, v)?;
        let mt = tree_of(p, Some(*main))?;
        let out = kopano_core::merge::merge_trees(&p.store, tree_of(p, base)?, mt, tree_of(p, Some(*v))?, &|k: &str| mergeable(k))?;
        Ok(out.conflicts.is_empty() && tree::diff(&p.store, mt, out.root)?.is_empty())
    })()
    .unwrap_or(false)
}

/// Every version `account` made itself (not a merge) under `b` is in `main`.
fn own_edits_in(p: &Proj, b: &Id, main: &Id, account: &str) -> bool {
    let (Ok(hb), Ok(hm)) = (project::history(&p.store, &[*b]), project::history(&p.store, &[*main])) else { return false };
    let in_main: BTreeSet<Id> = hm.into_iter().map(|(id, _)| id).collect();
    hb.iter().filter(|(_, v)| v.cert.body.account == account && v.parents().is_ok_and(|ps| ps.len() < 2)).all(|(id, _)| in_main.contains(id))
}

/// Whose work a version is: for an approval (a merge into main), the person whose version was
/// approved; otherwise its signer.
fn author_of(p: &Proj, v: &Id) -> String {
    let Ok(ver) = project::load_version(&p.store, v) else { return String::new() };
    match ver.parents().ok().as_deref() {
        Some([_, theirs]) => project::load_version(&p.store, theirs).map(|t| t.cert.body.account.clone()).unwrap_or_else(|_| ver.cert.body.account.clone()),
        _ => ver.cert.body.account.clone(),
    }
}

/// Two approvers approved at the same moment and this device's main update lost (`wf::tips` keeps
/// one): the approval was already decided here, so it is applied again on top of the main that
/// won, when that needs no new decision. A clash between the two leaves it offered again.
/// Returns true when main moved. Call under `p.write`.
fn reapply_lost(node: &Node, p: &Proj) -> Result<bool> {
    let (t, forks) = tips(p)?;
    let Some((main_id, main_u)) = t.get(MAIN).cloned() else { return Ok(false) };
    let mut main = Id::parse(&main_u.version)?;
    let m = members(p)?;
    if !m.is_approver(&node.device.cert.body.account) || !m.device_ok(&node.device.cert) {
        return Ok(false);
    }
    let mine: Vec<Id> = forks.iter().filter(|f| p.store.has(f) && project::load_version(&p.store, f).is_ok_and(|v| v.cert.body.device == node.device.cert.body.device)).copied().collect();
    let (mut prev, mut moved) = ((main_id, main_u.seq), false);
    for f in mine {
        if adds_nothing(p, &f, &main) {
            continue;
        }
        let base = project::common_ancestor(&p.store, &main, &f)?;
        let out = kopano_core::merge::merge_trees(&p.store, tree_of(p, base)?, tree_of(p, Some(main))?, tree_of(p, Some(f))?, &|k: &str| mergeable(k))?;
        if !out.conflicts.is_empty() {
            continue; // a new decision: it stays offered again
        }
        let (nv, _) = wf::approve_with(&p.store, &p.project, &main, &f, &|_: &str| true, &|k: &str| mergeable(k), &node.device.cert, &node.device.key)?;
        let seen = wf::member_tips(&p.store, &p.review.members());
        let id = wf::update_ref(&p.store, &p.project, MAIN, &nv, Some(prev), &seen, &node.device.cert, &node.device.key)?;
        p.store.flush()?;
        learn(p, &[id], &[])?;
        eprintln!("[kopanod] {}: an approval made here crossed another approver's; applied it again on the main that won ({})", p.project.id.short(), nv.short());
        prev = (id, prev.1 + 1);
        main = nv;
        moved = true;
    }
    Ok(moved)
}

/// The ref this device's edits go to: main for the owner, the account's branch for anyone else.
pub fn my_ref(node: &Node, p: &Proj) -> String {
    let me = &node.device.cert.body.account;
    if *me == p.project.genesis.owner {
        MAIN.to_string()
    } else {
        wf::branch_of(me)
    }
}

/// After the window's head moved here (an edit saved, or changes brought in): sign it into this
/// device's ref. A member whose window simply shows main signs nothing. True when it signed.
pub fn sign_mine(node: &Node, p: &Proj) -> Result<bool> {
    if !on(p) {
        return Ok(false);
    }
    let Some(head) = p.head() else { return Ok(false) };
    let (t, _) = tips(p)?;
    let name = my_ref(node, p);
    if name != MAIN {
        if !members(p)?.device_ok(&node.device.cert) {
            bail!("you are no longer on this project");
        }
        if let Some(m) = version_of(&t, MAIN) {
            if is_anc(p, &head, &m) {
                return Ok(false);
            }
            // the window holds exactly main and his branch offers nothing more (his work came
            // into main by another way): show main. A branch that still offers something (he
            // undid it in the window) is signed as usual, so the undone work is not approved.
            if tree_of(p, Some(head))? == tree_of(p, Some(m))? && version_of(&t, &name).map_or(true, |b| adds_nothing(p, &b, &m)) {
                p.set_head(Some(m))?;
                return Ok(false);
            }
        }
    }
    if version_of(&t, &name) == Some(head) {
        return Ok(false);
    }
    let prev = t.get(&name).map(|(id, u)| (*id, u.seq));
    let seen = wf::member_tips(&p.store, &p.review.members());
    let id = wf::update_ref(&p.store, &p.project, &name, &head, prev, &seen, &node.device.cert, &node.device.key)?;
    p.store.flush()?;
    learn(p, &[id], &[])?;
    Ok(true)
}

/// Bring one version into the window (saving the window's own edit first), with the same retry
/// as live projects when Photo refuses an update built on an older snapshot.
/// `keep_mine`: a member taking main in keeps his own value where main changed the same setting
/// (see `keep_mine`), so the approver decides that clash, not the merge.
fn bring(node: &Node, p: &Proj, v: &Id, keep_mine_on_clash: bool) -> Result<Option<(&'static str, usize)>> {
    let before = p.head();
    let mut tries = 0;
    let (repo_head, r) = loop {
        let mut repo = p.repo(&node.device)?;
        match p.with_app(&node.device, |app| repo.integrate_checked(app, v)) {
            Err(e) if tries < 8 && kopano_lab::photo::is_refused(&e) => {
                tries += 1;
                tokio::task::block_in_place(|| std::thread::sleep(Duration::from_millis(100 * tries)));
            }
            r => break (repo.head, r?),
        }
    };
    p.set_head(repo_head)?;
    Ok(match r {
        Integrated::AlreadyHad if repo_head != before => Some(("saved an edit made meanwhile", 0)),
        Integrated::AlreadyHad => None,
        Integrated::FastForward { .. } => Some(("fast-forward", 0)),
        Integrated::Merged { conflicts, .. } => {
            if keep_mine_on_clash && !conflicts.is_empty() {
                keep_mine(node, p, &conflicts)?;
            }
            Some(("merged", conflicts.len()))
        }
    })
}

/// `into` with the settings a clash names (`fields`, as the merge labels them) taken from `from`.
fn take_fields(into: &Item, from: &Item, fields: &[String]) -> Item {
    if fields.iter().any(|f| f == "whole item") || into.kind != from.kind {
        return from.clone();
    }
    let mut state = into.state.clone();
    for (path, _, val) in differences(&into.state, &from.state) {
        if fields.contains(&label(&path)) {
            if let Some(slot) = state.pointer_mut(&format!("/{}", path.replace('.', "/"))) {
                *slot = val;
            }
        }
    }
    Item { state, ..into.clone() }
}

/// S8 found a member's own value replaced by main's when his window took main in: the merge
/// settles a clashing setting the same way on every device, so his -15 became main's 15 in his
/// own window, the approver saw no clash, and his work was gone. In an approve project the
/// member keeps his own value (his window, his branch, with the clash record) and the approver
/// decides with "Keep yours" / "Take theirs". Call under `p.write`, after the merge.
fn keep_mine(node: &Node, p: &Proj, conflicts: &[kopano_core::merge::Conflict]) -> Result<()> {
    let Some(head) = p.head() else { return Ok(()) };
    let root = tree_of(p, Some(head))?;
    let (mut changes, mut items) = (vec![], vec![]);
    for c in conflicts {
        let Some(mine) = c.mine else { continue }; // he removed it: the merge keeps data
        let mine = Item::get(&p.store, &mine)?;
        let now = tree::get(&p.store, root, &c.key)?.map(|i| Item::get(&p.store, &i)).transpose()?;
        let want = match &now {
            Some(n) => take_fields(n, &mine, &c.fields),
            None => mine.clone(),
        };
        if now.as_ref() != Some(&want) {
            changes.push((c.key.clone(), Some(want.put(&p.store)?)));
            items.push((c.key.clone(), want));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    let new_root = tree::update(&p.store, root, changes)?;
    let v = project::commit(&p.store, &p.project, vec![head], new_root, "Kept my own values where main changed the same settings (for the approver)", &node.device.cert, &node.device.key)?;
    // the window first: if it cannot take them, the head stays what the window shows
    p.with_app(&node.device, |app| {
        let info = app.info();
        for (k, it) in &items {
            if info.owns(&it.kind) {
                app.apply(k, Some(it))?;
            }
        }
        Ok(())
    })?;
    p.set_head(Some(v))
}

/// Clashes a member's branch carries as records (his window took main in and kept his own value,
/// `keep_mine`) that main still differs on: (item key, fields, main's item, his item).
fn record_clashes(p: &Proj, main: &Id, v: &Id) -> Result<Vec<(String, Vec<String>, Item, Item)>> {
    let base = project::common_ancestor(&p.store, main, v)?;
    let (bt, vt, mt) = (tree_of(p, base)?, tree_of(p, Some(*v))?, tree_of(p, Some(*main))?);
    let mut out = vec![];
    for (k, _, after) in tree::diff(&p.store, bt, vt)? {
        let Some(rid) = after.filter(|_| k.starts_with(CONFLICT_PREFIX)) else { continue };
        if tree::get(&p.store, mt, &k)?.is_some() {
            continue;
        }
        let rec = Item::get(&p.store, &rid)?;
        let Some(key) = rec.state["key"].as_str() else { continue };
        let fields: Vec<String> = rec.state["fields"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        let get = |t: Option<Id>| -> Result<Option<Item>> { tree::get(&p.store, t, key)?.map(|i| Item::get(&p.store, &i)).transpose() };
        let (Some(mi), Some(vi)) = (get(mt)?, get(vt)?) else { continue };
        if take_fields(&mi, &vi, &fields) != mi {
            out.push((key.to_string(), fields, mi, vi));
        }
    }
    Ok(out)
}

/// Bring the window to what it should show: the owner's to main; a member's to his branch with
/// the latest main merged in (or main itself when nothing of his waits). Call under `p.write`.
pub fn refresh(node: &Node, p: &Proj) -> Result<Option<(&'static str, usize)>> {
    if p.review.viewing().is_some() {
        return Ok(None); // the owner is looking at a member's version: wait until that ends
    }
    let reapplied = reapply_lost(node, p)?;
    let (t, _) = tips(p)?;
    let Some(main) = version_of(&t, MAIN) else { return Ok(None) };
    let mut wants = vec![];
    if my_ref(node, p) != MAIN {
        if let Some(b) = version_of(&t, &my_ref(node, p)).filter(|b| !is_anc(p, b, &main)) {
            wants.push(b); // my own work from my other device
        }
    }
    wants.push(main);
    let mut out: Option<(&'static str, usize)> = None;
    if p.head().is_none() {
        // a window that just joined never speaks first: it takes main as it is, and only what
        // the person changes after that is theirs (photos only this window has stay in it)
        adopt(node, p, &main)?;
        out = Some(("fast-forward", 0));
    }
    for w in wants {
        if p.head().is_some_and(|h| is_anc(p, &w, &h)) {
            continue;
        }
        let keep = w == main && my_ref(node, p) != MAIN;
        if let Some((how, c)) = bring(node, p, &w, keep)? {
            out = Some(match out {
                Some((_, c0)) => ("merged", c0 + c),
                None => (how, c),
            });
        }
    }
    if sign_mine(node, p)? && out.is_none() {
        out = Some(("saved an edit made meanwhile", 0));
    }
    if reapplied && out.is_none() {
        out = Some(("approved again", 0));
    }
    Ok(out)
}

/// Show `v` in a window that has no head yet, without saving the window's state first.
fn adopt(node: &Node, p: &Proj, v: &Id) -> Result<()> {
    let want = tree::entries(&p.store, tree_of(p, Some(*v))?)?;
    p.with_app(&node.device, |app| {
        let have = app.snapshot()?;
        let info = app.info();
        for (k, id) in &want {
            let item = Item::get(&p.store, id)?;
            if info.owns(&item.kind) && have.get(k) != Some(&item) {
                app.apply(k, Some(&item))?;
            }
        }
        Ok(())
    })?;
    p.set_head(Some(*v))
}

// ---- sync ------------------------------------------------------------------------------------

/// The versions every known ref points at (forks included, so nothing becomes unreachable).
pub(crate) fn ref_versions(p: &Proj) -> Vec<Id> {
    let mut v: BTreeSet<Id> = p.review.refs().iter().filter_map(|r| wf::load_ref(&p.store, r).ok()).filter_map(|s| Id::parse(&s.update.body.version).ok()).filter(|v| p.store.has(v)).collect();
    v.extend(p.head());
    v.into_iter().collect()
}

/// Ref updates no other known update replaces: what a watcher says it has.
fn ref_leaves(p: &Proj) -> Vec<Id> {
    let refs = p.review.refs();
    let replaced: BTreeSet<String> = refs.iter().filter_map(|r| wf::load_ref(&p.store, r).ok()).filter_map(|s| s.update.body.previous).collect();
    refs.into_iter().filter(|r| !replaced.contains(&r.hex())).collect()
}

/// Every update reachable from `leaves` through `previous`.
fn ref_chain(store: &dyn ObjectStore, leaves: &[Id]) -> BTreeSet<Id> {
    let mut out = BTreeSet::new();
    let mut stack = leaves.to_vec();
    while let Some(id) = stack.pop() {
        if !out.insert(id) {
            continue;
        }
        if let Ok(s) = wf::load_ref(store, &id) {
            stack.extend(s.update.body.previous.and_then(|x| Id::parse(&x).ok()));
        }
    }
    out
}

/// The watch request of an approve-mode project.
pub fn watch_request(p: &Proj) -> Value {
    let hex = |v: Vec<Id>| v.iter().map(Id::hex).collect::<Vec<_>>();
    json!({"op": "watch", "have": hex(ref_versions(p)), "have_refs": hex(ref_leaves(p)), "have_members": hex(wf::member_tips(&p.store, &p.review.members()))})
}

fn ids(v: &Value, k: &str) -> Vec<Id> {
    v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).collect()
}

/// One watcher of an approve-mode project: what it has, so each push carries only what is new.
pub struct Pusher {
    versions: Vec<Id>,
    refs: BTreeSet<Id>,
    members: BTreeSet<Id>,
}

impl Pusher {
    pub fn new(p: &Proj, req: &Value) -> Self {
        let versions = ids(req, "have").into_iter().filter(|h| p.store.has(h)).collect();
        let refs = ref_chain(&p.store, &ids(req, "have_refs"));
        let members = wf::member_closure(&p.store, &ids(req, "have_members"));
        Pusher { versions, refs, members }
    }

    /// The next push (note, compressed payload), or None when the watcher has everything.
    pub fn next(&mut self, p: &Proj) -> Result<Option<(Value, Vec<u8>)>> {
        let heads = ref_versions(p);
        let refs: Vec<Id> = p.review.refs().into_iter().filter(|r| !self.refs.contains(r)).collect();
        let members: Vec<Id> = p.review.members().into_iter().filter(|m| !self.members.contains(m)).collect();
        let new_versions = heads.iter().any(|h| !self.versions.contains(h));
        if refs.is_empty() && members.is_empty() && !new_versions {
            return Ok(None);
        }
        let mut extra: Vec<Vec<u8>> = vec![];
        for id in members.iter().chain(refs.iter()) {
            extra.push(p.store.get(id)?);
        }
        let (records, raw, z) = crate::compact_payload_of(p, &self.versions, &heads, &extra)?;
        let main = tips(p).ok().and_then(|(t, _)| version_of(&t, MAIN)).or(p.head());
        let note = json!({"head": main.map(|h| h.hex()), "ms": now_ms(), "records": records, "raw": raw,
            "refs": refs.iter().map(Id::hex).collect::<Vec<_>>(), "members": members.iter().map(Id::hex).collect::<Vec<_>>()});
        self.versions = heads;
        self.refs.extend(refs);
        self.members.extend(members);
        Ok(Some((note, z)))
    }
}

/// A push arrived for an approve-mode project: keep it, then bring the window to what it should
/// show. Returns (how, clashes, photos changed in the window) when the window changed.
pub async fn take(node: &Node, p: &Proj, note: &Value, z: &[u8]) -> Result<Option<(&'static str, usize, usize)>> {
    let raw = note.get("raw").and_then(Value::as_u64).unwrap_or(0);
    let out = {
        let _w = p.write.lock().await;
        let before = p.head();
        kopano_core::wire::decode_compact(&p.store, z, raw)?;
        // objects first, the files that name them second (as for the head)
        p.store.flush()?;
        let new = learn(p, &ids(note, "refs"), &ids(note, "members"))?;
        let r = refresh(node, p)?;
        if new || r.is_some() {
            // pass it on: a teammate who watches us but not the sender gets it too
            let _ = p.heads.send(p.head().unwrap_or(p.project.id));
        }
        r.map(|(how, c)| (how, c, p.photos_changed(before, p.head())))
    };
    notify(node, p);
    Ok(out)
}

// ---- what the panel shows ----------------------------------------------------------------------

/// A person's name by account (from the certificates seen), else the account.
fn name_of(node: &Node, p: &Proj, account: &str) -> String {
    if node.device.cert.body.account == account {
        return node.device.cert.body.name.clone();
    }
    if let Some(n) = p.names().values().find(|n| n.get("account").and_then(Value::as_str) == Some(account)).and_then(|n| n.get("name").and_then(Value::as_str).map(str::to_string)) {
        return n;
    }
    // the certificates inside what they signed (the genesis, ref updates) name them too
    if let Ok(g) = p.store.get(&p.project.id).and_then(|b| Ok(serde_json::from_slice::<project::SignedGenesis>(&b)?)) {
        if g.cert.body.account == account {
            return g.cert.body.name;
        }
    }
    p.review.refs().iter().filter_map(|r| wf::load_ref(&p.store, r).ok()).find(|s| s.cert.body.account == account).map(|s| s.cert.body.name).unwrap_or_else(|| account.to_string())
}

fn photo_of(key: &str) -> String {
    key.rsplit_once('#').map(|(b, _)| b.to_string()).unwrap_or_else(|| key.to_string())
}

fn tree_of(p: &Proj, v: Option<Id>) -> Result<Option<Id>> {
    match v {
        Some(v) => project::load_version(&p.store, &v)?.tree(),
        None => Ok(None),
    }
}

fn photos_text(n: usize) -> String {
    if n == 1 {
        "changed 1 photo".into()
    } else {
        format!("changed {n} photos")
    }
}

/// Leaf settings that differ between two states: (path, a, b).
fn differences(a: &Value, b: &Value) -> Vec<(String, Value, Value)> {
    fn flat(v: &Value, pre: &str, out: &mut BTreeMap<String, Value>) {
        match v {
            Value::Object(m) => m.iter().for_each(|(k, x)| flat(x, &if pre.is_empty() { k.clone() } else { format!("{pre}.{k}") }, out)),
            _ => {
                out.insert(pre.to_string(), v.clone());
            }
        }
    }
    let (mut fa, mut fb) = (BTreeMap::new(), BTreeMap::new());
    flat(a, "", &mut fa);
    flat(b, "", &mut fb);
    let keys: BTreeSet<&String> = fa.keys().chain(fb.keys()).collect();
    keys.into_iter()
        .filter_map(|k| {
            let (x, y) = (fa.get(k).cloned().unwrap_or(Value::Null), fb.get(k).cloned().unwrap_or(Value::Null));
            (x != y).then(|| (k.clone(), x, y))
        })
        .collect()
}

/// One member's work waiting for review: what they changed since the main they built on, and
/// where main changed the same settings since (the clashes the approver decides).
fn pending_entry(node: &Node, p: &Proj, account: &str, v: &Id, main: &Id) -> Result<Value> {
    let base = project::common_ancestor(&p.store, main, v)?;
    let (bt, vt, mt) = (tree_of(p, base)?, tree_of(p, Some(*v))?, tree_of(p, Some(*main))?);
    let mut photos: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut keys = vec![];
    for (k, before, after) in tree::diff(&p.store, bt, vt)? {
        if k.starts_with(CONFLICT_PREFIX) {
            continue;
        }
        let old = before.map(|i| Item::get(&p.store, &i)).transpose()?;
        let new = after.map(|i| Item::get(&p.store, &i)).transpose()?;
        let fields = match (&old, &new) {
            (_, Some(n)) => kopano_core::jsonmerge::describe(old.as_ref().map(|o| &o.state), &n.state),
            (Some(_), None) => vec!["removed".to_string()],
            _ => vec![],
        };
        photos.entry(photo_of(&k)).or_default().extend(fields.into_iter().map(Value::from));
        keys.push(k);
    }
    let out = kopano_core::merge::merge_trees(&p.store, bt, mt, vt, &|k: &str| mergeable(k))?;
    let mut clashes = vec![];
    for c in &out.conflicts {
        let (m, t) = (c.mine.map(|i| Item::get(&p.store, &i)).transpose()?, c.theirs.map(|i| Item::get(&p.store, &i)).transpose()?);
        let (ms, ts) = (m.map(|i| i.state).unwrap_or(Value::Null), t.map(|i| i.state).unwrap_or(Value::Null));
        let settings: Vec<Value> = differences(&ms, &ts).into_iter().map(|(path, a, b)| json!({"setting": path, "main": a, "member": b})).collect();
        clashes.push(json!({"key": c.key, "photo": photo_of(&c.key), "fields": c.fields, "settings": settings}));
    }
    for (key, fields, mi, vi) in record_clashes(p, main, v)? {
        if out.conflicts.iter().any(|c| c.key == key) {
            continue;
        }
        let whole = fields.iter().any(|f| f == "whole item");
        let settings: Vec<Value> = differences(&mi.state, &vi.state).into_iter().filter(|(path, _, _)| whole || fields.contains(&label(path))).map(|(path, a, b)| json!({"setting": path, "main": a, "member": b})).collect();
        clashes.push(json!({"key": key, "photo": photo_of(&key), "fields": fields, "settings": settings}));
    }
    let n = photos.len();
    Ok(json!({"account": account, "name": name_of(node, p, account), "version": v.hex(), "photos": n, "summary": photos_text(n),
        "changes": photos.into_iter().map(|(ph, f)| json!({"photo": ph, "fields": f})).collect::<Vec<_>>(), "keys": keys, "clashes": clashes}))
}

/// Everything the panel shows for an approve-mode project.
pub fn review(node: &Node, p: &Proj) -> Result<Value> {
    let (t, forks) = tips(p)?;
    let m = members(p)?;
    let me = node.device.cert.body.account.clone();
    let owner = p.project.genesis.owner.clone();
    let main = version_of(&t, MAIN);
    let mut pending = vec![];
    let mut declined = vec![];
    if let (true, Some(main)) = (m.is_approver(&me), main) {
        for (name, (_, u)) in &t {
            let Some(acct) = name.strip_prefix("branch/") else { continue };
            let Ok(v) = Id::parse(&u.version) else { continue };
            if adds_nothing(p, &v, &main) {
                continue;
            }
            if version_of(&t, &wf::review_of(acct)).is_some_and(|d| still_declined(p, &v, &d, &main)) {
                declined.push(json!({"account": acct, "name": name_of(node, p, acct), "version": v.hex()}));
                continue;
            }
            pending.push(pending_entry(node, p, acct, &v, &main)?);
        }
        // two approvers at once: the version that lost is offered again
        for f in forks.iter().filter(|f| p.store.has(f) && !adds_nothing(p, f, &main)) {
            let acct = author_of(p, f);
            // the same work still waits on its own branch: listed there already
            if version_of(&t, &wf::branch_of(&acct)).is_some_and(|b| !adds_nothing(p, &b, &main) && project::load_version(&p.store, f).ok().and_then(|v| v.parents().ok()).is_some_and(|ps| ps.last().is_some_and(|x| is_anc(p, x, &b)))) {
                continue;
            }
            let mut e = pending_entry(node, p, &acct, f, &main)?;
            e["again"] = json!(true);
            pending.push(e);
        }
    }
    let mine = if me == owner {
        Value::Null
    } else {
        let b = version_of(&t, &wf::branch_of(&me));
        let me = me.as_str();
        let state = match (b, main) {
            (None, _) => "none",
            (Some(b), Some(mv)) if is_anc(p, &b, &mv) => "approved",
            // his own edits are all in main (approved, by way of another approval): approved;
            // an edit of his that main lacks but adds nothing (he undid it) waits for nobody
            (Some(b), Some(mv)) if adds_nothing(p, &b, &mv) => {
                if own_edits_in(p, &b, &mv, &me) {
                    "approved"
                } else {
                    "none"
                }
            }
            (Some(b), Some(mv)) if version_of(&t, &wf::review_of(&me)).is_some_and(|d| still_declined(p, &b, &d, &mv)) => "declined",
            (Some(_), _) => "waiting",
        };
        let photos = match (b, main, state) {
            (Some(b), Some(mv), "waiting" | "declined") => {
                let base = project::common_ancestor(&p.store, &mv, &b)?;
                p.photos_changed(base, Some(b))
            }
            _ => 0,
        };
        json!({"state": state, "version": b.map(|b| b.hex()), "photos": photos, "summary": photos_text(photos)})
    };
    let approvers: Vec<String> = std::iter::once(owner.clone()).chain(m.map.iter().filter(|(a, i)| **a != owner && (i.approver || i.role == project::Role::Admin)).map(|(a, _)| a.clone())).collect();
    let viewing = p.review.viewing().map(|v| json!({"account": v.account, "name": name_of(node, p, &v.account), "version": v.version.hex(), "side": v.side}));
    Ok(json!({"mode": "approve", "owner": owner, "owner_name": name_of(node, p, &owner), "is_owner": me == owner, "approver": m.is_approver(&me),
        "approvers": approvers, "main": main.map(|v| v.hex()), "pending": pending, "declined": declined, "mine": mine, "viewing": viewing}))
}

/// Tell the panel when what it shows changed.
pub fn notify(node: &Node, p: &Proj) {
    if !on(p) {
        return;
    }
    let Ok(r) = review(node, p) else { return };
    let s = r.to_string();
    let mut last = p.review.last.lock().unwrap_or_else(|e| e.into_inner());
    if *last != s {
        *last = s;
        drop(last);
        let mut ev = json!({"event": "review", "project": p.project.id.hex()});
        if let (Some(o), Some(r)) = (ev.as_object_mut(), r.as_object()) {
            o.extend(r.clone());
        }
        node.emit(ev);
    }
}

// ---- what approvers and owners do ---------------------------------------------------------------

/// The account a request names: an account, or a name the certificates showed.
fn account_named(node: &Node, p: &Proj, who: &str, m: &Members) -> Result<String> {
    if m.is_member(who) {
        return Ok(who.to_string());
    }
    let by_name: Vec<String> = m.map.keys().chain(std::iter::once(&m.owner)).filter(|a| name_of(node, p, a).eq_ignore_ascii_case(who)).cloned().collect();
    match by_name.as_slice() {
        [a] => Ok(a.clone()),
        _ => bail!("{who} is not on this project"),
    }
}

/// The version of `account`'s work to act on: the one named (their branch tip or one of its
/// versions, or a version offered again), else their branch tip.
fn work_of(p: &Proj, t: &Tips, forks: &[Id], account: &str, version: Option<&str>) -> Result<Id> {
    let tip = version_of(t, &wf::branch_of(account));
    let Some(v) = version else { return tip.with_context(|| "nothing of theirs is waiting") };
    let v = match Id::parse(v) {
        Ok(v) => v,
        Err(_) => {
            // a short id, as events and the panel show it
            let cands: Vec<Id> = tip.into_iter().chain(forks.iter().copied()).filter(|c| c.hex().starts_with(v)).collect();
            *cands.first().context("no such version")?
        }
    };
    if forks.contains(&v) || tip.is_some_and(|t| is_anc(p, &v, &t)) {
        return Ok(v);
    }
    bail!("that version is not one of {account}'s")
}

/// Stop showing a member's version in the owner's window: the window shows main again (nothing
/// is saved: saves were paused while looking). Call under `p.write`.
fn end_view(node: &Node, p: &Proj) -> Result<()> {
    let Some(v) = p.review.viewing() else { return Ok(()) };
    let head_tree = tree_of(p, p.head())?;
    p.with_app(&node.device, |app| {
        let have = app.snapshot()?;
        for k in &v.keys {
            let want = tree::get(&p.store, head_tree, k)?.map(|i| Item::get(&p.store, &i)).transpose()?;
            if have.get(k) != want.as_ref() {
                app.apply(k, want.as_ref())?;
            }
        }
        Ok(())
    })?;
    *p.review.viewing.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// Show a member's version of the photos they changed in this window ("member"), or main's
/// ("main"), or stop ("off"). While shown, this window's saves wait, so nothing reaches main.
pub async fn view(node: &Node, p: &Proj, who: &str, version: Option<&str>, side: &str) -> Result<Value> {
    let _w = p.write.lock().await;
    if side == "off" {
        end_view(node, p)?;
        let r = refresh(node, p)?;
        drop(_w);
        if r.is_some() {
            let _ = p.heads.send(p.head().unwrap_or(p.project.id));
        }
        notify(node, p);
        return Ok(json!({"viewing": null}));
    }
    let m = members(p)?;
    if !m.is_approver(&node.device.cert.body.account) {
        bail!("Only the owner, or someone the owner made an approver, reviews changes");
    }
    let account = account_named(node, p, who, &m)?;
    let (t, forks) = tips(p)?;
    let v = work_of(p, &t, &forks, &account, version)?;
    let main = version_of(&t, MAIN).context("no main yet")?;
    if p.review.viewing().is_none() {
        // first keep what this window has (saves pause from here on)
        let mut repo = p.repo(&node.device)?;
        if p.with_app(&node.device, |app| repo.save(app, p.binding().edit_message()))?.is_some() {
            p.set_head(repo.head)?;
            sign_mine(node, p)?;
            let _ = p.heads.send(p.head().unwrap_or(p.project.id));
        }
    }
    let base = project::common_ancestor(&p.store, &main, &v)?;
    let keys: Vec<String> = tree::diff(&p.store, tree_of(p, base)?, tree_of(p, Some(v))?)?.into_iter().map(|(k, _, _)| k).filter(|k| !k.starts_with(CONFLICT_PREFIX)).collect();
    let src = if side == "member" { tree_of(p, Some(v))? } else { tree_of(p, p.head())? };
    *p.review.viewing.lock().unwrap_or_else(|e| e.into_inner()) = Some(Viewing { account: account.clone(), version: v, keys: keys.clone(), side: side.to_string() });
    let r = p.with_app(&node.device, |app| {
        let have = app.snapshot()?;
        let info = app.info();
        let mut n = 0;
        for k in &keys {
            let want = tree::get(&p.store, src, k)?.map(|i| Item::get(&p.store, &i)).transpose()?;
            if want.as_ref().is_some_and(|w| !info.owns(&w.kind)) {
                continue;
            }
            if have.get(k) != want.as_ref() {
                app.apply(k, want.as_ref())?;
                n += 1;
            }
        }
        Ok(n)
    });
    if let Err(e) = &r {
        let _ = end_view(node, p);
        *p.review.viewing.lock().unwrap_or_else(|e| e.into_inner()) = None;
        bail!("could not show it in the window: {e:#}");
    }
    drop(_w);
    notify(node, p);
    Ok(json!({"viewing": {"account": account, "version": v.hex(), "side": side, "items": keys.len(), "applied": r?}}))
}

/// `approve {member, version, prefer, choices}`: take a member's work into main. Clashes follow
/// `choices` (item key or photo -> "member" | "main"), else `prefer` (default: the member's).
pub async fn approve(node: &Node, p: &Proj, who: &str, version: Option<&str>, prefer: Option<&str>, choices: &Map<String, Value>) -> Result<Value> {
    let w = p.write.lock().await;
    let m = members(p)?;
    if !m.is_approver(&node.device.cert.body.account) || !m.device_ok(&node.device.cert) {
        bail!("Only the owner, or someone the owner made an approver, can approve changes");
    }
    end_view(node, p)?;
    let account = account_named(node, p, who, &m)?;
    let (t, forks) = tips(p)?;
    let v = work_of(p, &t, &forks, &account, version)?;
    let (main_id, main_u) = t.get(MAIN).cloned().context("no main yet")?;
    let main = Id::parse(&main_u.version)?;
    let default_member = prefer.unwrap_or("member") != "main";
    let pick = |key: &str| -> bool {
        let c = choices.get(key).or_else(|| choices.get(&photo_of(key))).and_then(Value::as_str);
        c.map_or(default_member, |c| c != "main")
    };
    let recorded = record_clashes(p, &main, &v)?;
    let (mut nv, mut clashes) = wf::approve_with(&p.store, &p.project, &main, &v, &pick, &|k: &str| mergeable(k), &node.device.cert, &node.device.key)?;
    // clashes his window recorded (keep_mine): his value is in his work; "Keep yours" puts main's back
    let root = tree_of(p, Some(nv))?;
    let mut changes = vec![];
    for (key, fields, mi, _) in &recorded {
        clashes += 1;
        if pick(key) {
            continue;
        }
        if let Some(now) = tree::get(&p.store, root, key)?.map(|i| Item::get(&p.store, &i)).transpose()? {
            let want = take_fields(&now, mi, fields);
            if want != now {
                changes.push((key.clone(), Some(want.put(&p.store)?)));
            }
        }
    }
    if !changes.is_empty() {
        let new_root = tree::update(&p.store, root, changes)?;
        nv = project::commit(&p.store, &p.project, vec![nv], new_root, "Clash decided: kept main's value", &node.device.cert, &node.device.key)?;
        p.store.flush()?;
    }
    let seen = wf::member_tips(&p.store, &p.review.members());
    let id = wf::update_ref(&p.store, &p.project, MAIN, &nv, Some((main_id, main_u.seq)), &seen, &node.device.cert, &node.device.key)?;
    p.store.flush()?;
    learn(p, &[id], &[])?;
    let before = p.head();
    let r = refresh(node, p)?;
    let photos = p.photos_changed(before, p.head());
    drop(w);
    let _ = p.heads.send(p.head().unwrap_or(p.project.id));
    if r.is_some() {
        node.emit(json!({"event": "synced", "project": p.project.id.hex(), "from": name_of(node, p, &account), "head": p.head().map(|h| h.short()),
            "how": "approved", "clashes": 0, "photos": photos, "now_ms": now_ms()}));
    }
    notify(node, p);
    Ok(json!({"main": nv.hex(), "approved": v.hex(), "member": account, "clashes_decided": clashes, "photos": photos}))
}

/// `decline {member, version}`: main stays as it is; the member's branch keeps the work and the
/// member sees "Declined" (a signed mark everyone receives).
pub async fn decline(node: &Node, p: &Proj, who: &str, version: Option<&str>) -> Result<Value> {
    let w = p.write.lock().await;
    let m = members(p)?;
    if !m.is_approver(&node.device.cert.body.account) || !m.device_ok(&node.device.cert) {
        bail!("Only the owner, or someone the owner made an approver, can decline changes");
    }
    end_view(node, p)?;
    let account = account_named(node, p, who, &m)?;
    let (t, forks) = tips(p)?;
    let v = work_of(p, &t, &forks, &account, version)?;
    let name = wf::review_of(&account);
    let prev = t.get(&name).map(|(id, u)| (*id, u.seq));
    let seen = wf::member_tips(&p.store, &p.review.members());
    let id = wf::update_ref(&p.store, &p.project, &name, &v, prev, &seen, &node.device.cert, &node.device.key)?;
    p.store.flush()?;
    learn(p, &[id], &[])?;
    refresh(node, p)?;
    drop(w);
    let _ = p.heads.send(p.head().unwrap_or(p.project.id));
    notify(node, p);
    Ok(json!({"declined": v.hex(), "member": account}))
}

/// `set-approver {account, approver}`: the owner (or an admin) lets a member approve, or stops
/// them. A signed members entry; then main is sealed so the old members no longer count.
pub async fn set_approver(node: &Node, p: &Proj, who: &str, approver: bool) -> Result<Value> {
    let w = p.write.lock().await;
    let m = members(p)?;
    let me = node.device.cert.body.account.clone();
    if !m.is_admin(&me) {
        bail!("Only the owner can choose who approves");
    }
    let account = account_named(node, p, who, &m)?;
    if account == m.owner {
        bail!("the owner always approves");
    }
    let info = m.map.get(&account).cloned().context("not a member")?;
    let tips_m = wf::member_tips(&p.store, &p.review.members());
    let e = wf::add_member_entry(&p.store, &p.project, tips_m, MemberOp::Set { account: account.clone(), role: info.role, lanes: info.lanes, approver }, &node.device.cert, &node.device.key)?;
    p.store.flush()?;
    learn(p, &[], &[e])?;
    let (t, _) = tips(p)?;
    if let Some((mid, mu)) = t.get(MAIN).cloned() {
        let seen = wf::member_tips(&p.store, &p.review.members());
        let s = wf::seal_main(&p.store, &p.project, (mid, &mu), &seen, &node.device.cert, &node.device.key)?;
        p.store.flush()?;
        learn(p, &[s], &[])?;
    }
    drop(w);
    let _ = p.heads.send(p.head().unwrap_or(p.project.id));
    notify(node, p);
    Ok(json!({"account": account, "approver": approver}))
}

/// Sign several members changes (people invited or removed on ThabangVision, share.rs), then
/// seal main like `set_approver`. Only an admin's entries count (workflow.rs); the caller checks.
pub async fn change_members(node: &Node, p: &Proj, ops: Vec<MemberOp>) -> Result<usize> {
    if ops.is_empty() {
        return Ok(0);
    }
    let w = p.write.lock().await;
    for op in &ops {
        let tips_m = wf::member_tips(&p.store, &p.review.members());
        let e = wf::add_member_entry(&p.store, &p.project, tips_m, op.clone(), &node.device.cert, &node.device.key)?;
        p.store.flush()?;
        learn(p, &[], &[e])?;
    }
    let (t, _) = tips(p)?;
    if let Some((mid, mu)) = t.get(MAIN).cloned() {
        let seen = wf::member_tips(&p.store, &p.review.members());
        let s = wf::seal_main(&p.store, &p.project, (mid, &mu), &seen, &node.device.cert, &node.device.key)?;
        p.store.flush()?;
        learn(p, &[s], &[])?;
    }
    drop(w);
    let _ = p.heads.send(p.head().unwrap_or(p.project.id));
    notify(node, p);
    Ok(ops.len())
}

/// Status lines for `status`: who approves, what waits, what this person's work is at.
pub fn status_fields(node: &Node, p: &Proj) -> Value {
    if !on(p) {
        return json!({"mode": "live"});
    }
    match review(node, p) {
        Ok(r) => json!({"mode": "approve", "review": r}),
        Err(e) => json!({"mode": "approve", "review_error": format!("{e:#}")}),
    }
}

// ---- live projects: clashes and a choice ---------------------------------------------------

/// Open clashes in a live project's head: both values of each clashing setting.
pub fn clashes(p: &Proj) -> Result<Vec<Value>> {
    let mut out = vec![];
    for (k, id) in tree::entries(&p.store, tree_of(p, p.head())?)? {
        if !k.starts_with(CONFLICT_PREFIX) {
            continue;
        }
        let rec = Item::get(&p.store, &id)?;
        let side = |s: &str| -> Option<Item> { rec.state[s].as_str().and_then(|h| Id::parse(h).ok()).and_then(|i| Item::get(&p.store, &i).ok()) };
        let (a, b) = (side("mine"), side("theirs"));
        let (sa, sb) = (a.as_ref().map(|i| i.state.clone()).unwrap_or(Value::Null), b.as_ref().map(|i| i.state.clone()).unwrap_or(Value::Null));
        let fields: Vec<String> = rec.state["fields"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        let settings: Vec<Value> = differences(&sa, &sb)
            .into_iter()
            .filter(|(path, _, _)| fields.iter().any(|f| f == "whole item") || fields.contains(&label(path)))
            .map(|(path, x, y)| json!({"setting": path, "a": x, "b": y}))
            .collect();
        let item = rec.state["key"].as_str().unwrap_or_default().to_string();
        out.push(json!({"clash": k, "key": item, "photo": photo_of(&item), "fields": fields, "settings": settings}));
    }
    Ok(out)
}

/// The generic label kopano-core's merge writes for a setting path ("light.exposure" ->
/// "Light exposure"), to find which settings a clash record names.
fn label(path: &str) -> String {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let tail = &parts[parts.len().saturating_sub(2)..];
    let text = tail.iter().map(|s| s.replace('_', " ")).collect::<Vec<_>>().join(" ");
    let mut c = text.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => text,
    }
}

/// `resolve {clash, pick: "a" | "b"}` in a live project: the clashing settings take side a's or
/// b's value (other settings stay as merged), the clash record goes, and everyone gets it.
pub async fn resolve(node: &Node, p: &Proj, clash: &str, pick: &str) -> Result<Value> {
    let w = p.write.lock().await;
    // the window's newest edit first
    let mut repo = p.repo(&node.device)?;
    if p.with_app(&node.device, |app| repo.save(app, p.binding().edit_message()))?.is_some() {
        p.set_head(repo.head)?;
    }
    let head = p.head().context("nothing here yet")?;
    let root = tree_of(p, Some(head))?;
    let rid = tree::get(&p.store, root, clash)?.context("that clash is already settled")?;
    let rec = Item::get(&p.store, &rid)?;
    let key = rec.state["key"].as_str().context("clash record without its item")?.to_string();
    let side = rec.state[if pick == "a" { "mine" } else { "theirs" }].as_str().and_then(|h| Id::parse(h).ok());
    let other = rec.state[if pick == "a" { "theirs" } else { "mine" }].as_str().and_then(|h| Id::parse(h).ok());
    let fields: Vec<String> = rec.state["fields"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
    let now = tree::get(&p.store, root, &key)?.map(|i| Item::get(&p.store, &i)).transpose()?;
    let chosen = side.map(|i| Item::get(&p.store, &i)).transpose()?;
    let new_item: Option<Item> = match (&now, &chosen, other.map(|i| Item::get(&p.store, &i)).transpose()?) {
        (Some(n), Some(c), Some(o)) if !fields.iter().any(|f| f == "whole item") => {
            let mut state = n.state.clone();
            for (path, val, _) in differences(&c.state, &o.state) {
                if fields.contains(&label(&path)) {
                    let ptr = format!("/{}", path.replace('.', "/"));
                    match state.pointer_mut(&ptr) {
                        Some(slot) => *slot = val,
                        None => continue,
                    }
                }
            }
            Some(Item { state, ..n.clone() })
        }
        _ => chosen.clone(),
    };
    let changes = vec![(key.clone(), new_item.as_ref().map(|i| i.put(&p.store)).transpose()?), (clash.to_string(), None)];
    let new_root = tree::update(&p.store, root, changes)?;
    let v = project::commit(&p.store, &p.project, vec![head], new_root, "Clash settled", &node.device.cert, &node.device.key)?;
    p.set_head(Some(v))?;
    p.with_app(&node.device, |app| {
        if new_item.as_ref().map_or(true, |i| app.info().owns(&i.kind)) {
            app.apply(&key, new_item.as_ref())?;
        }
        Ok(())
    })?;
    drop(w);
    let _ = p.heads.send(v);
    node.emit(json!({"event": "saved", "project": p.project.id.hex(), "head": v.short(), "photos": 1, "now_ms": now_ms()}));
    Ok(json!({"settled": clash, "key": key, "version": v.hex()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Binding;
    use kopano_core::identity::{DeviceKey, Provider};
    use kopano_core::store::RedbStore;
    use kopano_lab::ToyEditor;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kopano-approve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("projects")).unwrap();
        d
    }

    fn home(provider: &Provider, dir: &Path, account: &str, name: &str) -> Result<()> {
        let key = DeviceKey::generate();
        std::fs::write(dir.join("device.key"), key.to_bytes())?;
        let cert = provider.issue(account, name, &key, "laptop", 3_600_000)?;
        std::fs::write(dir.join("cert.json"), serde_json::to_vec(&cert)?)?;
        Ok(())
    }

    struct Team {
        dirs: Vec<PathBuf>,
        nodes: Vec<Arc<Node>>,
        pid: Id,
    }

    impl Team {
        fn p(&self, i: usize) -> Arc<Proj> {
            self.nodes[i].project(&self.pid.hex()).unwrap()
        }
        /// `to` learns what `from` has (one push, as the watch channel would carry it).
        async fn sync(&self, from: usize, to: usize) -> Result<Option<(&'static str, usize, usize)>> {
            let (pf, pt) = (self.p(from), self.p(to));
            let mut pusher = Pusher::new(&pf, &watch_request(&pt));
            match pusher.next(&pf)? {
                Some((note, z)) => take(&self.nodes[to], &pt, &note, &z).await,
                None => Ok(None),
            }
        }
        async fn both(&self) -> Result<()> {
            for _ in 0..2 {
                self.sync(0, 1).await?;
                self.sync(1, 0).await?;
            }
            Ok(())
        }
        fn get(&self, i: usize, key: &str, field: &str) -> f64 {
            let app = self.p(i).app().unwrap();
            app.photos[key].state.pointer(&format!("/{}", field.replace('.', "/"))).and_then(Value::as_f64).unwrap()
        }
        async fn set(&self, i: usize, key: &str, field: &str, v: f64) -> Result<()> {
            self.nodes[i].set(&self.p(i), key, field, json!(v)).await?;
            Ok(())
        }
        fn main(&self, i: usize) -> Option<Id> {
            version_of(&tips(&self.p(i)).unwrap().0, MAIN)
        }
        fn review(&self, i: usize) -> Value {
            review(&self.nodes[i], &self.p(i)).unwrap()
        }
    }

    /// Alice (owner) and Member1 (editor), an "I approve changes" project of 3 toy photos.
    async fn team(tag: &str) -> Result<Team> {
        let provider = Provider::generate();
        let dirs = vec![temp(&format!("{tag}-a")), temp(&format!("{tag}-b"))];
        home(&provider, &dirs[0], "acct-alice", "Alice")?;
        home(&provider, &dirs[1], "acct-m1", "Member1")?;
        let a = Node::open(&dirs[0])?;
        let b = Node::open(&dirs[1])?;
        let members = BTreeMap::from([("acct-m1".to_string(), project::Member { role: project::Role::Editor, lanes: vec![] })]);
        let p = a.create_project_mode("shoot", Binding::Toy, members, Some("approve"))?;
        // the owner's window: three photos, saved as the first main
        std::fs::write(p.dir.join("app.json"), serde_json::to_vec(&ToyEditor::demo(3).photos)?)?;
        {
            let _w = p.write.lock().await;
            let mut repo = p.repo(&a.device)?;
            let mut app = p.app()?;
            repo.save(&mut app, "first upload")?;
            p.set_head(repo.head)?;
            assert!(sign_mine(&a, &p)?, "the owner's first save is main");
        }
        // Member1 joins (what `join` does after fetching the genesis)
        let dir = dirs[1].join("projects").join(&p.project.id.hex()[..8]);
        std::fs::create_dir_all(&dir)?;
        {
            let store = RedbStore::open(dir.join("kopano.redb"))?;
            store.put_verified(&p.project.id, &p.store.get(&p.project.id)?)?;
            store.flush()?;
        }
        std::fs::write(dir.join("project"), p.project.id.hex())?;
        std::fs::write(dir.join("binding.json"), serde_json::to_vec(&Binding::Toy)?)?;
        std::fs::write(dir.join("app.json"), b"{}")?;
        let pb = Arc::new(Proj::open(&dir)?);
        b.projects.write().unwrap().insert(p.project.id, pb);
        let t = Team { dirs, nodes: vec![a, b], pid: p.project.id };
        t.sync(0, 1).await?;
        assert_eq!(t.p(1).head(), t.main(0), "the member's window shows main after joining");
        assert_eq!(t.p(1).app()?.photos.len(), 3);
        Ok(t)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_members_edit_waits_for_approval_then_main_follows() -> Result<()> {
        let t = team("approve").await?;
        let main0 = t.main(0);
        t.set(1, "photo-0001", "develop.exposure", 1.25).await?;
        t.set(1, "photo-0002", "rating", 4.0).await?;
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), 1.25, "the member sees his own edit at once");
        t.both().await?;
        assert_eq!(t.main(0), main0, "main did not move");
        assert_eq!(t.p(0).head(), main0, "the owner's window still shows main");
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0);
        let r = t.review(0);
        assert_eq!(r["pending"].as_array().unwrap().len(), 1, "{r}");
        let e = &r["pending"][0];
        assert_eq!(e["account"], "acct-m1");
        assert_eq!(e["name"], "Member1");
        assert_eq!(e["photos"], 2);
        assert_eq!(e["summary"], "changed 2 photos");
        assert_eq!(t.review(1)["mine"]["state"], "waiting");
        let version = e["version"].as_str().unwrap().to_string();
        let out = approve(&t.nodes[0], &t.p(0), "Member1", Some(&version), None, &Map::new()).await?;
        assert_eq!(out["clashes_decided"], 0);
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 1.25, "the owner's window follows main");
        assert_eq!(t.get(0, "photo-0002", "rating"), 4.0);
        assert_eq!(t.p(0).head(), t.main(0));
        t.both().await?;
        assert_eq!(t.main(1), t.main(0));
        assert_eq!(t.p(1).head(), t.main(0), "the member's window is main again");
        assert_eq!(t.review(1)["mine"]["state"], "approved");
        assert_eq!(t.review(0)["pending"].as_array().unwrap().len(), 0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_decline_leaves_main_and_keeps_the_members_work() -> Result<()> {
        let t = team("decline").await?;
        let main0 = t.main(0);
        t.set(1, "photo-0001", "develop.exposure", -2.0).await?;
        t.both().await?;
        decline(&t.nodes[0], &t.p(0), "acct-m1", None).await?;
        assert_eq!(t.main(0), main0);
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0);
        assert_eq!(t.review(0)["pending"].as_array().unwrap().len(), 0);
        assert_eq!(t.review(0)["declined"].as_array().unwrap().len(), 1);
        t.both().await?;
        assert_eq!(t.review(1)["mine"]["state"], "declined");
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), -2.0, "the member's branch keeps the work");
        // main moves on: his window takes it in, and the declined work is not offered again
        t.set(0, "photo-0002", "develop.contrast", 5.0).await?;
        t.both().await?;
        assert_eq!(t.get(1, "photo-0002", "develop.contrast"), 5.0);
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), -2.0);
        assert_eq!(t.review(1)["mine"]["state"], "declined");
        assert_eq!(t.review(0)["pending"].as_array().unwrap().len(), 0, "{}", t.review(0));
        // working on: waiting again, with the earlier work in it
        t.set(1, "photo-0000", "develop.contrast", 10.0).await?;
        t.both().await?;
        assert_eq!(t.review(1)["mine"]["state"], "waiting");
        assert_eq!(t.review(0)["pending"][0]["photos"], 2);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_owners_edits_reach_a_member_with_work_waiting() -> Result<()> {
        let t = team("follow").await?;
        t.set(1, "photo-0001", "develop.exposure", 0.5).await?;
        t.both().await?;
        t.set(0, "photo-0002", "develop.contrast", 30.0).await?;
        let r = t.sync(0, 1).await?;
        assert!(r.is_some_and(|(how, c, _)| how == "merged" && c == 0), "{r:?}");
        assert_eq!(t.get(1, "photo-0002", "develop.contrast"), 30.0, "main's change shows in the member's window");
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), 0.5, "his own work stays");
        t.both().await?;
        let r = t.review(0);
        assert_eq!(r["pending"][0]["photos"], 1, "only his own photo is his change: {r}");
        approve(&t.nodes[0], &t.p(0), "acct-m1", None, None, &Map::new()).await?;
        assert_eq!(t.get(0, "photo-0002", "develop.contrast"), 30.0);
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.5);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clashes_are_decided_per_photo() -> Result<()> {
        let t = team("clash").await?;
        t.set(1, "photo-0001", "develop.exposure", 1.0).await?;
        t.set(1, "photo-0002", "develop.exposure", 1.0).await?;
        t.sync(1, 0).await?;
        t.set(0, "photo-0001", "develop.exposure", -1.0).await?;
        t.set(0, "photo-0002", "develop.exposure", -1.0).await?;
        // the member has not seen main's change yet: the clash is the approver's to decide
        let r = t.review(0);
        let clashes = r["pending"][0]["clashes"].as_array().unwrap();
        assert_eq!(clashes.len(), 2, "{r}");
        assert_eq!(clashes[0]["settings"][0]["setting"], "develop.exposure");
        assert_eq!(clashes[0]["settings"][0]["main"], -1.0);
        assert_eq!(clashes[0]["settings"][0]["member"], 1.0);
        let choices = serde_json::from_value::<Map<String, Value>>(json!({"photo-0001": "main", "photo-0002": "member"}))?;
        let out = approve(&t.nodes[0], &t.p(0), "acct-m1", None, None, &choices).await?;
        assert_eq!(out["clashes_decided"], 2);
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), -1.0);
        assert_eq!(t.get(0, "photo-0002", "develop.exposure"), 1.0);
        t.both().await?;
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), -1.0);
        assert_eq!(t.get(1, "photo-0002", "develop.exposure"), 1.0);
        assert_eq!(t.p(1).head(), t.main(0));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_member_who_is_not_an_approver_cannot_move_main() -> Result<()> {
        let t = team("refused").await?;
        let main0 = t.main(0);
        t.set(1, "photo-0001", "develop.exposure", 3.0).await?;
        let e = approve(&t.nodes[1], &t.p(1), "acct-m1", None, None, &Map::new()).await.unwrap_err();
        assert!(format!("{e}").contains("Only the owner"), "{e}");
        assert!(decline(&t.nodes[1], &t.p(1), "acct-m1", None).await.is_err());
        assert!(set_approver(&t.nodes[1], &t.p(1), "acct-m1", true).await.is_err());
        // forged: he signs a main update himself and it reaches the owner
        let pb = t.p(1);
        let (tb, _) = tips(&pb)?;
        let (mid, mu) = tb.get(MAIN).cloned().unwrap();
        let head = pb.head().unwrap();
        let seen = wf::member_tips(&pb.store, &pb.review.members());
        let forged = wf::update_ref(&pb.store, &pb.project, MAIN, &head, Some((mid, mu.seq)), &seen, &t.nodes[1].device.cert, &t.nodes[1].device.key)?;
        pb.store.flush()?;
        learn(&pb, &[forged], &[])?;
        t.both().await?;
        assert!(t.p(0).review.refs().contains(&forged), "the forged update arrived");
        assert_eq!(t.main(0), main0, "and main did not move");
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0);
        assert_eq!(t.main(1), main0, "not even on his own computer");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_owner_makes_a_member_an_approver() -> Result<()> {
        let t = team("grant").await?;
        set_approver(&t.nodes[0], &t.p(0), "acct-m1", true).await?;
        t.both().await?;
        assert!(t.review(1)["approver"].as_bool().unwrap());
        t.set(1, "photo-0001", "develop.exposure", 2.0).await?;
        approve(&t.nodes[1], &t.p(1), "acct-m1", None, None, &Map::new()).await?;
        t.both().await?;
        assert_eq!(t.main(0), t.main(1));
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 2.0, "the owner's window follows the approver's main");
        // and the owner takes it back: his next approval is refused
        set_approver(&t.nodes[0], &t.p(0), "acct-m1", false).await?;
        t.both().await?;
        t.set(1, "photo-0001", "develop.exposure", 4.0).await?;
        assert!(approve(&t.nodes[1], &t.p(1), "acct-m1", None, None, &Map::new()).await.is_err());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_owner_views_a_members_version_without_saving_it() -> Result<()> {
        let t = team("view").await?;
        t.set(1, "photo-0001", "develop.exposure", 1.5).await?;
        t.both().await?;
        let main0 = t.main(0);
        view(&t.nodes[0], &t.p(0), "acct-m1", None, "member").await?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 1.5, "the member's version shows");
        assert_eq!(t.review(0)["viewing"]["side"], "member");
        view(&t.nodes[0], &t.p(0), "acct-m1", None, "main").await?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0, "before");
        view(&t.nodes[0], &t.p(0), "acct-m1", None, "member").await?;
        view(&t.nodes[0], &t.p(0), "", None, "off").await?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0);
        assert_eq!(t.main(0), main0, "looking saved nothing");
        assert_eq!(t.p(0).head(), main0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_is_kept_across_a_restart() -> Result<()> {
        let t = team("restart").await?;
        t.set(1, "photo-0001", "develop.exposure", 0.75).await?;
        t.both().await?;
        decline(&t.nodes[0], &t.p(0), "acct-m1", None).await?;
        t.both().await?;
        let (ra, rb) = (t.review(0), t.review(1));
        let pid = t.pid;
        let dirs = t.dirs.clone();
        drop(t);
        let nodes = vec![Node::open(&dirs[0])?, Node::open(&dirs[1])?];
        let t = Team { dirs, nodes, pid };
        assert_eq!(t.review(0), ra);
        assert_eq!(t.review(1), rb);
        assert_eq!(t.review(1)["mine"]["state"], "declined");
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), 0.75);
        // and work goes on
        t.set(1, "photo-0001", "develop.exposure", 0.8).await?;
        t.both().await?;
        approve(&t.nodes[0], &t.p(0), "acct-m1", None, None, &Map::new()).await?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.8);
        Ok(())
    }

    /// Signed in with ThabangVision (share.rs): Carol accepts an invite, the owner's device signs
    /// her into the members log and her computer is let in; removed on the website, she is
    /// refused again. Peers decide from the log, never from the website.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn someone_invited_on_thabangvision_is_let_in_only_while_on_the_log() -> Result<()> {
        let provider = Provider::generate();
        let (da, dc) = (temp("tvlog-a"), temp("tvlog-c"));
        home(&provider, &da, "acct-alice", "Alice")?;
        home(&provider, &dc, "acct-carol", "Carol")?;
        let (a, c) = (Node::open(&da)?, Node::open(&dc)?);
        let p = a.create_project_mode("shoot", Binding::Toy, BTreeMap::new(), Some("approve"))?;
        let carol = c.device.cert.clone();
        let rid: [u8; 32] = hex::decode(&carol.body.device)?.try_into().unwrap();
        assert!(p.check_peer(&carol, &rid).is_err(), "not invited: refused");
        assert!(!p.has_teammates());
        let accepted = BTreeMap::from([("acct-carol".to_string(), "editor".to_string())]);
        assert_eq!(change_members(&a, &p, crate::share::plan(&members(&p)?, &accepted, &BTreeSet::new())).await?, 1);
        p.check_peer(&carol, &rid)?;
        assert!(p.has_teammates(), "previews are made ahead for her now");
        assert!(!members(&p)?.is_approver("acct-carol"));
        let synced = BTreeSet::from(["acct-carol".to_string()]);
        assert!(crate::share::plan(&members(&p)?, &accepted, &synced).is_empty(), "nothing signed twice");
        assert_eq!(change_members(&a, &p, crate::share::plan(&members(&p)?, &BTreeMap::new(), &synced)).await?, 1);
        assert!(p.check_peer(&carol, &rid).is_err(), "removed: refused");
        Ok(())
    }

    /// A certificate ThabangVision stopped renewing (computer unlinked, account suspended) is
    /// refused by peers once it expires, member or not.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_expired_certificate_is_refused_by_peers() -> Result<()> {
        let provider = Provider::generate();
        let da = temp("expired-a");
        home(&provider, &da, "acct-alice", "Alice")?;
        let a = Node::open(&da)?;
        let members = BTreeMap::from([("acct-m1".to_string(), project::Member { role: project::Role::Editor, lanes: vec![] })]);
        let p = a.create_project_mode("shoot", Binding::Toy, members, Some("approve"))?;
        let key = DeviceKey::generate();
        let rid: [u8; 32] = hex::decode(key.public_hex())?.try_into().unwrap();
        let old = provider.issue("acct-m1", "Member1", &key, "laptop", 1)?;
        std::thread::sleep(Duration::from_millis(5));
        let e = p.check_peer(&old, &rid).unwrap_err();
        assert!(format!("{e:#}").contains("expired"), "{e:#}");
        p.check_peer(&provider.issue("acct-m1", "Member1", &key, "laptop", 3_600_000)?, &rid)?;
        // a certificate from an issuer the project does not trust is refused too
        assert!(p.check_peer(&Provider::generate().issue("acct-m1", "Member1", &key, "laptop", 3_600_000)?, &rid).is_err());
        Ok(())
    }

    /// Live mode: both changed exposure at once, the merge kept a clash record; picking one value
    /// writes it, removes the record and leaves the other settings as merged.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_live_clash_is_settled_by_a_pick() -> Result<()> {
        let provider = Provider::generate();
        let dir = temp("resolve");
        home(&provider, &dir, "acct-alice", "Alice")?;
        let a = Node::open(&dir)?;
        let bkey = DeviceKey::generate();
        let bcert = provider.issue("acct-m1", "Member1", &bkey, "laptop", 3_600_000)?;
        let members = BTreeMap::from([("acct-m1".to_string(), project::Member { role: project::Role::Editor, lanes: vec![] })]);
        let p = a.create_project_mode("live", Binding::Toy, members, None)?;
        assert!(!on(&p));
        std::fs::write(p.dir.join("app.json"), serde_json::to_vec(&ToyEditor::demo(2).photos)?)?;
        let base = {
            let _w = p.write.lock().await;
            let mut repo = p.repo(&a.device)?;
            let mut app = p.app()?;
            repo.save(&mut app, "start")?;
            p.set_head(repo.head)?;
            repo.head.unwrap()
        };
        // Member1, from the same start: exposure -1 and contrast 20
        let mut theirs = p.app()?;
        let it = theirs.photos.get_mut("photo-0001").unwrap();
        it.state["develop"]["exposure"] = json!(-1.0);
        it.state["develop"]["contrast"] = json!(20.0);
        let mut rb = kopano_core::repo::Repo { store: &p.store, project: project::Project::load(&p.store, &p.project.id)?, head: Some(base), cert: bcert, key: bkey };
        let tv = rb.save(&mut theirs, "member edit")?.unwrap();
        // Alice: exposure +1, then Member1's edit comes in
        a.set(&p, "photo-0001", "develop.exposure", json!(1.0)).await?;
        {
            let _w = p.write.lock().await;
            let mut repo = p.repo(&a.device)?;
            let mut app = p.app()?;
            match repo.integrate(&mut app, &tv)? {
                Integrated::Merged { conflicts, .. } => assert_eq!(conflicts.len(), 1),
                _ => panic!("expected a merge"),
            }
            p.save_app(&app)?;
            p.set_head(repo.head)?;
        }
        let open = clashes(&p)?;
        assert_eq!(open.len(), 1, "{open:?}");
        let c = &open[0];
        assert_eq!(c["photo"], "photo-0001");
        assert_eq!(c["settings"].as_array().unwrap().len(), 1, "only the clashing setting: {c}");
        assert_eq!(c["settings"][0]["setting"], "develop.exposure");
        let (va, vb) = (c["settings"][0]["a"].as_f64().unwrap(), c["settings"][0]["b"].as_f64().unwrap());
        let want_b = vb;
        resolve(&a, &p, c["clash"].as_str().unwrap(), "b").await?;
        let app = p.app()?;
        assert_eq!(app.photos["photo-0001"].state["develop"]["exposure"], json!(want_b), "the pick (a was {va})");
        assert_eq!(app.photos["photo-0001"].state["develop"]["contrast"], json!(20.0), "the other setting stays as merged");
        assert!(clashes(&p)?.is_empty());
        assert!(resolve(&a, &p, c["clash"].as_str().unwrap(), "a").await.is_err(), "settled once");
        Ok(())
    }

    /// Link the window again (what a restarted kopanod or a restarted app does first).
    fn relink(t: &Team, i: usize) -> Result<ToyEditor> {
        let p = t.p(i);
        let mut app = p.app()?;
        tokio::task::block_in_place(|| p.reconcile(&t.nodes[i].device, &mut app))?;
        p.save_app(&app)?;
        Ok(app)
    }

    fn set_in_window(t: &Team, i: usize, key: &str, field: &str, v: Value) -> Result<()> {
        let p = t.p(i);
        let mut app = p.app()?;
        *app.photos.get_mut(key).unwrap().state.pointer_mut(&format!("/{}", field.replace('.', "/"))).unwrap() = v;
        p.save_app(&app)
    }

    /// S8: edits made in the window while kopanod was not running are the person's work: linking
    /// the window again keeps them (and the next save sends them for review). Old states (a
    /// fresh window, an old file) are still put back to the project's.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_linked_again_keeps_edits_made_while_unlinked() -> Result<()> {
        let t = team("relink").await?;
        t.set(1, "photo-0002", "develop.contrast", 12.0).await?; // now in his history
        set_in_window(&t, 1, "photo-0001", "develop.exposure", json!(0.9))?; // made offline
        set_in_window(&t, 1, "photo-0002", "develop.contrast", json!(0.0))?; // an older state
        relink(&t, 1)?;
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), 0.9, "new work stays in the window");
        assert_eq!(t.get(1, "photo-0002", "develop.contrast"), 12.0, "an old state is put back");
        let kept = std::fs::read(t.p(1).dir.join("kept.json")).map(|b| serde_json::from_slice::<Vec<String>>(&b).unwrap().len()).unwrap_or(0);
        assert_eq!(kept, 1, "the old state is kept aside as before");
        // the next save (as the poll does) sends it for review
        let p = t.p(1);
        {
            let _w = p.write.lock().await;
            let mut repo = p.repo(&t.nodes[1].device)?;
            let mut app = p.app()?;
            repo.save(&mut app, "edit")?;
            p.set_head(repo.head)?;
            sign_mine(&t.nodes[1], &p)?;
        }
        t.both().await?;
        assert_eq!(t.review(0)["pending"][0]["photos"], 2, "{}", t.review(0));
        Ok(())
    }

    /// S8: the owner's window still shows a member's version (she was looking at it when the
    /// window or kopanod went down, so the view was never ended): linking it again must put main
    /// back, never save his version into main as her own edit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_members_version_left_in_the_owners_window_is_not_approved() -> Result<()> {
        let t = team("leftview").await?;
        t.set(1, "photo-0001", "develop.exposure", 1.5).await?;
        t.both().await?;
        let main0 = t.main(0);
        view(&t.nodes[0], &t.p(0), "acct-m1", None, "member").await?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 1.5);
        *t.p(0).review.viewing.lock().unwrap() = None; // kopanod restarted: the view is forgotten
        relink(&t, 0)?;
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.0, "main is back in her window");
        let p = t.p(0);
        {
            let _w = p.write.lock().await;
            let mut repo = p.repo(&t.nodes[0].device)?;
            let mut app = p.app()?;
            assert!(repo.save(&mut app, "edit")?.is_none(), "nothing to save");
        }
        assert_eq!(t.main(0), main0, "main did not move");
        assert_eq!(t.review(0)["pending"].as_array().unwrap().len(), 1, "still waiting for her");
        Ok(())
    }

    /// S8: two approvers move main at the same moment. The update that loses is applied again by
    /// the device that made it (no new decision needed), so both count; nothing is offered again,
    /// nothing waits as "changed 0 photos", every window ends on one main.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn crossing_approvals_both_count() -> Result<()> {
        let t = team("cross").await?;
        set_approver(&t.nodes[0], &t.p(0), "acct-m1", true).await?;
        t.both().await?;
        // the same work, approved on both devices before either hears of the other
        t.set(1, "photo-0001", "develop.exposure", 0.8).await?;
        t.both().await?;
        approve(&t.nodes[0], &t.p(0), "acct-m1", None, None, &Map::new()).await?;
        approve(&t.nodes[1], &t.p(1), "acct-m1", None, None, &Map::new()).await?;
        for _ in 0..3 {
            t.both().await?;
        }
        assert_eq!(t.main(0), t.main(1), "one main");
        assert_eq!(t.get(0, "photo-0001", "develop.exposure"), 0.8);
        assert_eq!(t.get(1, "photo-0001", "develop.exposure"), 0.8);
        assert_eq!(t.review(0)["pending"].as_array().unwrap().len(), 0, "{}", t.review(0));
        assert_eq!(t.review(1)["pending"].as_array().unwrap().len(), 0, "{}", t.review(1));
        assert_eq!(t.review(1)["mine"]["state"], "approved");
        // different work: the owner edits main while Member1 (an approver) approves his own work
        t.set(1, "photo-0002", "rating", 4.0).await?;
        t.set(0, "photo-0000", "develop.contrast", 22.0).await?;
        approve(&t.nodes[1], &t.p(1), "acct-m1", None, None, &Map::new()).await?;
        for _ in 0..3 {
            t.both().await?;
        }
        assert_eq!(t.main(0), t.main(1), "one main after the fork");
        for i in 0..2 {
            assert_eq!(t.get(i, "photo-0002", "rating"), 4.0, "his approval counts on {i}");
            assert_eq!(t.get(i, "photo-0000", "develop.contrast"), 22.0, "her edit counts on {i}");
            assert!(t.review(i)["pending"].as_array().unwrap().is_empty(), "{}", t.review(i));
            assert_eq!(t.p(i).head(), t.main(i), "window {i} shows main");
        }
        assert_eq!(t.review(1)["mine"]["state"], "approved");
        Ok(())
    }

    /// S8: main's change reaches the member before his reaches the approver, on the same setting.
    /// His window keeps his own value (not main's), and the approver decides the clash.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_member_keeps_his_value_and_the_approver_decides() -> Result<()> {
        let t = team("keepmine").await?;
        for (round, (mine, hers, choice)) in [(-15.0, 15.0, "main"), (-7.0, 7.0, "member")].into_iter().enumerate() {
            t.set(1, "photo-0001", "develop.contrast", mine).await?;
            t.set(0, "photo-0001", "develop.contrast", hers).await?;
            t.sync(0, 1).await?; // he takes main in first
            assert_eq!(t.get(1, "photo-0001", "develop.contrast"), mine, "round {round}: his own value stays in his window");
            t.both().await?;
            let r = t.review(0);
            let c = &r["pending"][0]["clashes"];
            assert_eq!(c.as_array().map(Vec::len), Some(1), "round {round}: the approver sees the clash: {r}");
            assert_eq!(c[0]["settings"][0]["main"], json!(hers));
            assert_eq!(c[0]["settings"][0]["member"], json!(mine));
            let choices = serde_json::from_value::<Map<String, Value>>(json!({"photo-0001": choice}))?;
            approve(&t.nodes[0], &t.p(0), "acct-m1", None, None, &choices).await?;
            let want = if choice == "main" { hers } else { mine };
            t.both().await?;
            assert_eq!(t.get(0, "photo-0001", "develop.contrast"), want, "round {round}: main has the choice");
            assert_eq!(t.get(1, "photo-0001", "develop.contrast"), want, "round {round}: his window follows main");
            assert_eq!(t.p(1).head(), t.main(1));
            assert!(t.review(0)["pending"].as_array().unwrap().is_empty());
        }
        Ok(())
    }

    /// A member undoes his change in the window: his branch must not keep offering the undone work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_undone_change_is_not_offered() -> Result<()> {
        let t = team("undo").await?;
        t.set(1, "photo-0001", "develop.exposure", 0.8).await?;
        t.set(1, "photo-0001", "develop.exposure", 0.0).await?;
        t.both().await?;
        assert!(t.review(0)["pending"].as_array().unwrap().is_empty(), "{}", t.review(0));
        assert_eq!(t.review(1)["mine"]["state"], "none");
        Ok(())
    }

    #[test]
    fn a_live_project_keeps_its_genesis_bytes() -> Result<()> {
        let store = kopano_core::store::MemStore::default();
        let provider = Provider::generate();
        let key = DeviceKey::generate();
        let cert = provider.issue("a", "A", &key, "t", 3_600_000)?;
        let live = project::Project::create(&store, "x", vec![provider.public_hex()], &cert, &key, BTreeMap::new())?;
        assert!(!String::from_utf8(store.get(&live.id)?)?.contains("\"mode\""), "no mode written for live projects");
        assert!(!live.approve_mode());
        let named = project::Project::create_with_mode(&store, "x", vec![provider.public_hex()], &cert, &key, BTreeMap::new(), Some("live"))?;
        assert!(!named.approve_mode() && named.genesis.mode.is_none());
        let a = project::Project::create_with_mode(&store, "x", vec![provider.public_hex()], &cert, &key, BTreeMap::new(), Some("approve"))?;
        assert!(project::Project::load(&store, &a.id)?.approve_mode());
        assert!(project::Project::create_with_mode(&store, "x", vec![provider.public_hex()], &cert, &key, BTreeMap::new(), Some("other")).is_err());
        Ok(())
    }
}
