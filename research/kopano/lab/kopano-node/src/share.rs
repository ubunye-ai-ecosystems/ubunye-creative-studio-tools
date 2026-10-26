//! Projects shared through ThabangVision (09 section 2: "membership truth stays in the signed
//! members log; ThabangVision keeps a mirror for invites and my projects").
//!
//! - The owner shares a project: ThabangVision gets its name and id (`tv.json` in the project
//!   folder keeps ThabangVision's id for it).
//! - The owner invites by EMAIL. The answer is always "Invite sent.", whether or not that address
//!   has an account (no account enumeration). kopanod never stores or shows an email.
//! - The invited person sees the project under "Shared with me" and presses Join: kopanod tells
//!   ThabangVision they accepted, asks which computers the project's people use, and joins from
//!   the owner's computer. Nobody types an id.
//! - The owner's computer adds people who accepted to the project's signed members log (only an
//!   admin's entries count, workflow.rs). It asks ThabangVision when someone it does not know
//!   knocks, after an invite or a removal, and every few minutes. Peers then decide offline who
//!   may edit, from the log, as before; ThabangVision is never asked per connection.
//! - Leave (the person) and remove (the owner) go to ThabangVision first; the owner's computer
//!   then signs the removal into the log, so the person's devices are refused from then on.

use crate::tv::{self, Tv};
use crate::{approve, Binding, Node, Proj};
use anyhow::{bail, Context, Result};
use kopano_core::identity::{now_ms, DeviceCert};
use kopano_core::project::Role;
use kopano_core::workflow::MemberOp;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the owner's computer looks at ThabangVision's member list without a reason.
const MIRROR_EVERY: Duration = Duration::from_secs(300);
/// At most one question per project this often when strangers knock (they can knock in a loop).
const ASK_GAP: Duration = Duration::from_secs(4);
/// How long Join keeps knocking while the owner's computer signs the newcomer in.
const JOIN_WAIT: Duration = Duration::from_secs(60);

/// `tv.json` in a project folder: ThabangVision's id for it, and (owner) the accounts this device
/// signed into the log because ThabangVision listed them. Only those are ever taken out again,
/// so people added another way (lab ids) are never removed by the mirror.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Link {
    pub id: String,
    #[serde(default)]
    pub synced: BTreeSet<String>,
}

pub fn link(p: &Proj) -> Option<Link> {
    std::fs::read(p.dir.join("tv.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn save_link(p: &Proj, l: &Link) -> Result<()> {
    kopano_core::store::write_atomic(&p.dir.join("tv.json"), &serde_json::to_vec_pretty(l)?)
}

fn tv(node: &Node) -> Result<Tv> {
    Tv::for_home(&node.device.home)
}

fn me(node: &Node) -> String {
    node.device.cert.body.account.clone()
}

/// ThabangVision's roles (desktop-api.md section 3) as members-log roles: (role, may approve).
pub fn role_of(tv_role: &str) -> (Role, bool) {
    match tv_role {
        "owner" | "admin" => (Role::Admin, true),
        "approver" => (Role::Editor, true),
        "editor" => (Role::Editor, false),
        _ => (Role::Viewer, false),
    }
}

/// The changes that bring the log to ThabangVision's list. `listed`: account -> ThabangVision
/// role, for people who accepted (status active), the owner left out.
pub fn plan(log: &kopano_core::workflow::Members, listed: &BTreeMap<String, String>, synced: &BTreeSet<String>) -> Vec<MemberOp> {
    let mut ops = vec![];
    for (account, r) in listed {
        if *account == log.owner {
            continue;
        }
        let (role, approver) = role_of(r);
        let cur = log.map.get(account);
        if cur.map(|c| (c.role, c.approver)) != Some((role, approver)) {
            ops.push(MemberOp::Set { account: account.clone(), role, lanes: cur.map(|c| c.lanes.clone()).unwrap_or_default(), approver });
        }
    }
    for account in synced {
        if !listed.contains_key(account) && *account != log.owner && log.map.contains_key(account) {
            ops.push(MemberOp::Remove { account: account.clone() });
        }
    }
    ops
}

/// Owner's device: bring the signed members log to what ThabangVision lists. Returns how many
/// entries it signed.
pub async fn sync_members(node: &Node, p: &Proj) -> Result<usize> {
    let Some(mut l) = link(p) else { return Ok(0) };
    if !approve::on(p) {
        return Ok(0);
    }
    let log = approve::members(p)?;
    if !log.is_admin(&me(node)) {
        return Ok(0);
    }
    node.mirror_asked.lock().unwrap_or_else(|e| e.into_inner()).insert(p.project.id, Instant::now());
    let body = tv(node)?.call(&node.device.key, "GET", &tv::with_query("/api/kopano/projects/members", "project_id", &l.id)?, None).await?;
    follow_list(node, p, &mut l, &log, &body).await
}

/// `sync_members` once ThabangVision's member list (3d) is here.
async fn follow_list(node: &Node, p: &Proj, l: &mut Link, log: &kopano_core::workflow::Members, body: &Value) -> Result<usize> {
    let mut listed = BTreeMap::new();
    for m in body["members"].as_array().into_iter().flatten() {
        if let (Some(u), Some(r), Some("active")) = (m["user_id"].as_str(), m["role"].as_str(), m["status"].as_str()) {
            if r != "owner" {
                listed.insert(u.to_string(), r.to_string());
            }
        }
    }
    let ops = plan(log, &listed, &l.synced);
    let n = approve::change_members(node, p, ops.clone()).await?;
    let synced: BTreeSet<String> = listed.keys().cloned().collect();
    if synced != l.synced {
        l.synced = synced;
        save_link(p, l)?;
    }
    if n > 0 {
        eprintln!("[kopanod] {}: members log brought to ThabangVision's list ({n} change(s))", p.project.id.short());
        node.emit(json!({"event": "people", "project": p.project.id.hex()}));
    }
    Ok(n)
}

/// A device with a valid certificate knocked on a shared project and is not on it yet: if this
/// is the owner's device, ask ThabangVision once (people who just accepted), then check again.
pub async fn admit(node: &Arc<Node>, pid: &str, their: &DeviceCert, remote: &[u8; 32]) -> Option<Arc<Proj>> {
    let p = node.project(pid).ok()?;
    link(&p)?;
    if their.check(&p.project.genesis.providers, now_ms()).is_err() || their.body.device != hex::encode(remote) {
        return None;
    }
    if node.mirror_asked.lock().unwrap_or_else(|e| e.into_inner()).get(&p.project.id).is_some_and(|t| t.elapsed() < ASK_GAP) {
        return None;
    }
    if let Err(e) = sync_members(node, &p).await {
        eprintln!("[kopanod] could not ask ThabangVision who is on {}: {e:#}", p.project.id.short());
        return None;
    }
    p.check_peer(their, remote).ok().map(|_| p)
}

/// Owner's device: look at ThabangVision's lists every few minutes (people who accepted while
/// nobody knocked, removals made on the website).
pub fn spawn_mirror(node: Arc<Node>) {
    tokio::spawn(async move {
        loop {
            let ps: Vec<Arc<Proj>> = node.projects.read().unwrap().values().cloned().collect();
            for p in ps {
                if link(&p).is_none() || p.is_gone() || !approve::members(&p).is_ok_and(|m| m.is_admin(&me(&node))) {
                    continue;
                }
                if let Err(e) = sync_members(&node, &p).await {
                    eprintln!("[kopanod] {}: {e:#}", p.project.id.short());
                }
            }
            tokio::time::sleep(MIRROR_EVERY).await;
        }
    });
}

/// `share {project}`: put a project this person owns on ThabangVision, so people can be invited.
pub async fn share(node: &Arc<Node>, p: &Arc<Proj>) -> Result<Value> {
    if let Some(l) = link(p) {
        return Ok(json!({"shared": l.id}));
    }
    if p.project.genesis.owner != me(node) {
        bail!("Only the project's owner can share it.");
    }
    if !approve::on(p) {
        bail!("This project cannot take new people. Start a new project to share it.");
    }
    let tv = tv(node)?;
    let body = json!({"name": p.project.genesis.name, "kopano_project_id": p.project.id.hex()});
    let r = tv.signed(&node.device.key, "POST", "/api/kopano/projects", Some(&body)).await?;
    let id = if r.ok() {
        r.body["id"].as_str().context("ThabangVision did not answer with an id")?.to_string()
    } else if r.status == 409 {
        // already there (shared before this folder knew it)
        let list = tv.call(&node.device.key, "GET", "/api/kopano/projects", None).await?;
        let kp = p.project.id.hex();
        list["mine"].as_array().into_iter().flatten().find(|x| x["kopano_project_id"] == kp).and_then(|x| x["id"].as_str()).context("That project is already shared by someone else.")?.to_string()
    } else {
        eprintln!("[kopanod] share: {} {}", r.status, r.body);
        bail!("{}", tv::plain_error(&r));
    };
    save_link(p, &Link { id: id.clone(), synced: BTreeSet::new() })?;
    node.emit(json!({"event": "people", "project": p.project.id.hex()}));
    Ok(json!({"shared": id}))
}

/// `invite {project, email, role?}`: always "Invite sent." once the caller may invite.
pub async fn invite(node: &Arc<Node>, p: &Arc<Proj>, email: &str, role: Option<&str>) -> Result<Value> {
    let email = email.trim();
    if !email.contains('@') || email.len() > 254 || email.chars().any(char::is_whitespace) {
        bail!("Type the teammate's email address.");
    }
    let role = role.unwrap_or("editor");
    if !["editor", "viewer", "approver", "admin"].contains(&role) {
        bail!("Choose editor, viewer, approver or admin.");
    }
    let id = share(node, p).await?["shared"].as_str().context("not shared")?.to_string();
    tv(node)?.call(&node.device.key, "POST", "/api/kopano/projects/invite", Some(&json!({"project_id": id, "email": email, "role": role}))).await?;
    Ok(json!({"message": "Invite sent."}))
}

/// `people {project}`: who is on it, by display name (never an email), and who is online now.
pub async fn people(node: &Arc<Node>, p: &Arc<Proj>) -> Result<Value> {
    let online: BTreeSet<String> = {
        let names = p.names();
        p.online().keys().filter_map(|d| names.get(d).and_then(|n| n["account"].as_str()).map(String::from)).collect()
    };
    let Some(mut l) = link(p) else {
        return Ok(json!({"shared": false, "people": [{"user": me(node), "name": node.device.cert.body.name, "role": "owner", "status": "active", "me": true, "online": true}]}));
    };
    let body = tv(node)?.call(&node.device.key, "GET", &tv::with_query("/api/kopano/projects/members", "project_id", &l.id)?, None).await?;
    // the owner opening the list also brings the log to it (someone left on the website)
    if let Ok(log) = approve::members(p) {
        if approve::on(p) && log.is_admin(&me(node)) {
            follow_list(node, p, &mut l, &log, &body).await?;
        }
    }
    let log = approve::members(p).ok();
    let people: Vec<Value> = body["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let user = m["user_id"].as_str().unwrap_or_default();
            json!({
                "user": user, "name": m["name"], "avatar": m["avatar_url"], "role": m["role"], "status": m["status"], "me": m["is_me"],
                "online": m["is_me"] == true || online.contains(user),
                // signed into the project's members log: their computers can open it now
                "on_project": log.as_ref().is_some_and(|l| l.is_member(user)),
            })
        })
        .collect();
    Ok(json!({"shared": l.id, "people": people}))
}

/// `shared-projects`: "My projects" and "Shared with me" from ThabangVision, each marked with
/// whether it is on this computer already.
pub async fn list(node: &Arc<Node>) -> Result<Value> {
    let body = tv(node)?.call(&node.device.key, "GET", "/api/kopano/projects", None).await?;
    let here = |kp: &str| node.project(kp).ok().filter(|p| p.project.id.hex() == kp);
    let row = |x: &Value| {
        let kp = x["kopano_project_id"].as_str().unwrap_or_default();
        json!({"shared": x["id"], "name": x["name"], "project": kp, "role": x["role"], "status": x["status"], "here": here(kp).is_some()})
    };
    let mine: Vec<Value> = body["mine"].as_array().into_iter().flatten().map(row).collect();
    // a project this computer owns that lost its tv.json (copied folder): link it again
    for x in body["mine"].as_array().into_iter().flatten() {
        if let (Some(p), Some(id)) = (here(x["kopano_project_id"].as_str().unwrap_or_default()), x["id"].as_str()) {
            if link(&p).is_none() {
                save_link(&p, &Link { id: id.into(), synced: BTreeSet::new() })?;
            }
        }
    }
    let shared: Vec<Value> = body["shared"].as_array().into_iter().flatten().filter(|x| x["status"] == "invited" || x["status"] == "active").map(row).collect();
    Ok(json!({"mine": mine, "shared": shared}))
}

/// `join-shared {shared, app}`: open a project from "Shared with me". `shared` is ThabangVision's
/// id from `shared-projects` (the project id works too).
pub async fn join_shared(node: &Arc<Node>, shared: &str, binding: Binding) -> Result<Value> {
    let tv = tv(node)?;
    let key = &node.device.key;
    let list = tv.call(key, "GET", "/api/kopano/projects", None).await?;
    let item = list["shared"].as_array().into_iter().flatten().find(|x| x["id"] == shared || x["kopano_project_id"] == shared).cloned().context("That project is not shared with you.")?;
    let id = item["id"].as_str().context("no id")?.to_string();
    let kp = item["kopano_project_id"].as_str().context("no project")?.to_string();
    if !(kp.len() == 64 && kp.chars().all(|c| c.is_ascii_hexdigit())) {
        bail!("That project cannot be opened by this app.");
    }
    tv.call(key, "POST", "/api/kopano/projects/accept", Some(&json!({"project_id": id}))).await?;
    if let Ok(p) = node.project(&kp) {
        save_link(&p, &Link { id, ..link(&p).unwrap_or_default() })?;
        return Ok(json!({"project": kp, "name": item["name"], "already_here": true}));
    }
    // which computers to ask: the owner's first, most recently seen first; never this one
    let members = tv.call(key, "GET", &tv::with_query("/api/kopano/projects/members", "project_id", &id)?, None).await?;
    let owner = members["members"].as_array().into_iter().flatten().find(|m| m["role"] == "owner").and_then(|m| m["user_id"].as_str()).unwrap_or_default().to_string();
    let devs = tv.call(key, "GET", &tv::with_query("/api/kopano/projects/devices", "project_id", &id)?, None).await?;
    let mut devices: Vec<(bool, String, String)> = devs["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let k = d["device_key"].as_str()?.to_string();
            (k.len() == 64 && k != node.device.id()).then(|| (d["user_id"] != owner.as_str(), d["last_seen"].as_str().unwrap_or("").to_string(), k))
        })
        .collect();
    // owner first, then the newest last_seen (ISO times sort as text)
    devices.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    if devices.is_empty() {
        bail!("The project owner's computer has not been signed in yet. Try again when it is.");
    }
    let start = Instant::now();
    let mut waiting_for_owner = false;
    loop {
        for (_, _, dev) in &devices {
            match node.join(&kp, dev, binding.clone()).await {
                Ok(p) => {
                    save_link(&p, &Link { id: id.clone(), synced: BTreeSet::new() })?;
                    for (_, _, other) in &devices {
                        if other != dev {
                            let _ = node.add_teammate(p.clone(), other);
                        }
                    }
                    node.emit(json!({"event": "joined", "project": kp, "name": item["name"]}));
                    return Ok(json!({"project": kp, "name": item["name"]}));
                }
                Err(e) => {
                    let e = format!("{e:#}");
                    eprintln!("[kopanod] join {}: {}: {e}", &kp[..8], &dev[..8]);
                    waiting_for_owner |= e.contains("not a member");
                }
            }
        }
        if start.elapsed() > JOIN_WAIT {
            break;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    if waiting_for_owner {
        bail!("The owner's computer has not let you in yet. Try again in a minute.");
    }
    bail!("The project owner's computer is not online. Try again when it is.")
}

/// Find a project on this computer by its id or by ThabangVision's id for it.
fn local(node: &Node, which: &str) -> Option<Arc<Proj>> {
    if let Ok(p) = node.project(which) {
        return Some(p);
    }
    node.projects.read().unwrap().values().find(|p| link(p).is_some_and(|l| l.id == which)).cloned()
}

/// `leave {project}`: this person leaves a shared project (local id or ThabangVision's id). Its
/// files on this computer stay; it stops syncing and is not opened again.
pub async fn leave(node: &Arc<Node>, which: &str) -> Result<Value> {
    let p = local(node, which);
    if p.as_ref().is_some_and(|p| p.project.genesis.owner == me(node)) {
        bail!("You own this project, so you cannot leave it.");
    }
    let id = match p.as_ref().and_then(|p| link(p)) {
        Some(l) => l.id,
        None if p.is_none() => which.to_string(),
        None => bail!("That project was not shared through ThabangVision."),
    };
    let r = tv(node)?.signed(&node.device.key, "POST", "/api/kopano/projects/leave", Some(&json!({"project_id": id}))).await?;
    // 404: not in it any more (removed meanwhile): leaving here still makes sense
    if !r.ok() && r.status != 404 {
        bail!("{}", tv::plain_error(&r));
    }
    if let Some(p) = p {
        p.gone.store(true, std::sync::atomic::Ordering::Relaxed);
        // end its watches now (the loops see `gone` and let go of the project's files)
        for k in p.kicks.lock().unwrap().values() {
            k.notify_waiters();
        }
        std::fs::write(p.dir.join("left"), now_ms().to_string())?;
        node.projects.write().unwrap().remove(&p.project.id);
        node.left.lock().unwrap().insert(p.project.id, p.clone());
        node.emit(json!({"event": "left", "project": p.project.id.hex()}));
    }
    Ok(json!({"left": true}))
}

/// `remove-person {project, user}`: the owner takes someone off (ThabangVision's user id from
/// `people`). Their computers are refused as soon as this device has signed it.
pub async fn remove(node: &Arc<Node>, p: &Arc<Proj>, user: &str) -> Result<Value> {
    let l = link(p).context("That project was not shared through ThabangVision.")?;
    tv(node)?.call(&node.device.key, "POST", "/api/kopano/projects/remove", Some(&json!({"project_id": l.id, "user_id": user}))).await?;
    sync_members(node, p).await?;
    if approve::members(p)?.is_member(user) {
        // ThabangVision did not list them, but this device had not synced them either (added
        // another way): take them off the log directly
        approve::change_members(node, p, vec![MemberOp::Remove { account: user.to_string() }]).await?;
    }
    Ok(json!({"removed": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kopano_core::workflow::{MemberInfo, Members};

    fn log() -> Members {
        let mut m = Members { owner: "owner".into(), ..Default::default() };
        m.map.insert("owner".into(), MemberInfo { role: Role::Admin, lanes: vec![], approver: true });
        m.map.insert("lab-id".into(), MemberInfo { role: Role::Editor, lanes: vec![], approver: false });
        m
    }

    #[test]
    fn people_who_accepted_are_signed_in_with_their_role() {
        let listed = BTreeMap::from([("b".to_string(), "editor".to_string()), ("c".to_string(), "approver".to_string())]);
        let ops = plan(&log(), &listed, &BTreeSet::new());
        assert_eq!(ops.len(), 2);
        assert!(ops.contains(&MemberOp::Set { account: "b".into(), role: Role::Editor, lanes: vec![], approver: false }));
        assert!(ops.contains(&MemberOp::Set { account: "c".into(), role: Role::Editor, lanes: vec![], approver: true }));
    }

    #[test]
    fn nothing_is_signed_when_the_log_already_matches() {
        let mut m = log();
        m.map.insert("b".into(), MemberInfo { role: Role::Editor, lanes: vec!["photo".into()], approver: false });
        let listed = BTreeMap::from([("b".to_string(), "editor".to_string())]);
        assert!(plan(&m, &listed, &BTreeSet::from(["b".to_string()])).is_empty());
        // a role change keeps the lanes the owner gave them
        let listed = BTreeMap::from([("b".to_string(), "approver".to_string())]);
        assert_eq!(plan(&m, &listed, &BTreeSet::new()), vec![MemberOp::Set { account: "b".into(), role: Role::Editor, lanes: vec!["photo".into()], approver: true }]);
    }

    #[test]
    fn only_people_the_mirror_added_are_taken_out_and_never_the_owner() {
        let mut m = log();
        m.map.insert("b".into(), MemberInfo { role: Role::Editor, lanes: vec![], approver: false });
        // b left on the website; lab-id was never on ThabangVision; the owner is never removed
        let ops = plan(&m, &BTreeMap::new(), &BTreeSet::from(["b".to_string(), "owner".to_string()]));
        assert_eq!(ops, vec![MemberOp::Remove { account: "b".into() }]);
    }

    #[test]
    fn the_owner_listed_by_thabangvision_is_not_signed_again() {
        let listed = BTreeMap::from([("owner".to_string(), "admin".to_string())]);
        assert!(plan(&log(), &listed, &BTreeSet::new()).is_empty());
    }
}
