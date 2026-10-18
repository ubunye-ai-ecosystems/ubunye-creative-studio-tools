//! E17b: the v2 workflow (owner's main + personal branches + approvals + members log) under the
//! same kind of random abuse as sim_fuzz. Invariants checked at the end of every seed:
//!   W1 agreement: every honest current member sees the same main and the same branch tips
//!   W2 no loss: every edit an honest current member made is in main after the owner approves
//!      what is pending (approvers decide clashes, but nothing becomes unreachable)
//!   W3 safety: no main move by a non-approver, no branch move by anyone but its owner, no work
//!      from removed members or blocked devices, no lane violations, ever accepted
//!   W4 liveness: cut transfers never leave anyone stuck
//! Run: SIM_SEEDS=300 cargo test --release -p kopano-lab --test sim_workflow -- --nocapture

use anyhow::{bail, Context, Result};
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::identity::{DeviceCert, DeviceKey, Provider};
use kopano_core::project::{self, AssetRef, Item, Member, Project, Role};
use kopano_core::repo::{copy_version_closure, Repo};
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::tree;
use kopano_core::workflow::{self as wf, MemberOp, Members, MAIN};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const LANES: [(&str, &str); 3] = [("develop", "lab.develop"), ("selects", "lab.selects"), ("notes", "lab.notes")];
const FIELDS: [(&str, &str); 5] = [("develop", "exposure"), ("develop", "contrast"), ("develop", "temperature"), ("selects", "rating"), ("notes", "comment")];

#[derive(Clone, Default)]
struct App {
    items: BTreeMap<String, Item>,
}
impl App {
    fn import(&mut self, photo: &str) {
        let h = blake3::hash(photo.as_bytes()).to_hex().to_string();
        for (lane, kind) in LANES {
            let state = match lane {
                "develop" => json!({"exposure": 0, "contrast": 0, "temperature": 5500}),
                "selects" => json!({"rating": 0}),
                _ => json!({"comment": ""}),
            };
            let assets = if lane == "develop" { vec![AssetRef { hash: h.clone(), size: 1, role: "original".into() }] } else { vec![] };
            self.items.insert(format!("{photo}#{lane}"), Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets });
        }
    }
    fn photos(&self) -> Vec<String> {
        self.items.keys().filter_map(|k| k.split('#').next()).map(String::from).collect::<BTreeSet<_>>().into_iter().collect()
    }
    fn checkout(store: &dyn ObjectStore, version: &Id) -> Result<App> {
        let root = project::load_version(store, version)?.tree()?;
        let mut a = App::default();
        for (k, id) in tree::entries(store, root)? {
            let it = Item::get(store, &id)?;
            if LANES.iter().any(|(_, kind)| *kind == it.kind) {
                a.items.insert(k, it);
            }
        }
        Ok(a)
    }
}
impl Connector for App {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            app: "lab.sim2".into(),
            contract: CONTRACT_VERSION,
            kinds: LANES.iter().map(|(l, k)| KindInfo { kind: (*k).into(), lane: (*l).into(), schema: 1, mergeable: true }).collect(),
        }
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        Ok(self.items.clone())
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        match item {
            Some(i) => self.items.insert(key.into(), i.clone()),
            None => self.items.remove(key),
        };
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Behaviour {
    Honest,
    /// edits lanes it may not, moves main, moves other people's branches
    Cheat,
}

struct Peer {
    name: String,
    account: String,
    behaviour: Behaviour,
    store: MemStore,
    key: DeviceKey,
    cert: DeviceCert,
    refs: BTreeSet<Id>,
    members: BTreeSet<Id>,
    /// versions this peer verified (never re-verified)
    ok_versions: BTreeSet<Id>,
}

#[derive(Default, Debug)]
struct Stats {
    edits: u64,
    approvals: u64,
    clash_decisions: u64,
    catch_ups: u64,
    syncs: u64,
    cut_syncs: u64,
    member_changes: u64,
    cheats: u64,
    forks_reoffered: u64,
}

struct World {
    pid: Id,
    peers: Vec<Peer>,
    rng: StdRng,
    stats: Stats,
    /// (account, version) of every edit an honest peer made, for W2
    edits: Vec<(String, Id)>,
    photo_seq: u32,
}

fn policy(kind: &str) -> bool {
    LANES.iter().any(|(_, k)| *k == kind)
}

impl World {
    fn new(seed: u64) -> Result<World> {
        let mut rng = StdRng::seed_from_u64(seed);
        let provider = Provider::generate();
        let n = rng.gen_range(2..=5);
        let mut peers = vec![];
        let mut members = BTreeMap::new();
        for i in 0..=n {
            let name = if i == 0 { "owner".to_string() } else { format!("p{i}") };
            let key = DeviceKey::generate();
            let cert = provider.issue(&format!("acct-{name}"), &name, &key, "device", 86_400_000)?;
            let behaviour = if i > 0 && rng.gen_bool(0.2) { Behaviour::Cheat } else { Behaviour::Honest };
            if i > 0 {
                let (role, lanes): (Role, Vec<String>) = match rng.gen_range(0..4) {
                    0 | 1 => (Role::Editor, vec![]),
                    2 => (Role::Editor, vec!["develop".into()]),
                    _ => (Role::Commenter, vec![]),
                };
                members.insert(cert.body.account.clone(), Member { role, lanes });
            }
            peers.push(Peer { name, account: cert.body.account.clone(), behaviour, store: MemStore::default(), key, cert, refs: BTreeSet::new(), members: BTreeSet::new(), ok_versions: BTreeSet::new() });
        }
        let p = Project::create(&peers[0].store, "sim2", vec![provider.public_hex()], &peers[0].cert, &peers[0].key, members)?;
        let g = peers[0].store.get(&p.id)?;
        for q in &peers {
            q.store.put_verified(&p.id, &g)?;
        }
        let mut w = World { pid: p.id, peers, rng, stats: Stats::default(), edits: vec![], photo_seq: 0 };
        // the owner's first upload is main
        let mut app = App::default();
        for _ in 0..w.rng.gen_range(3..7) {
            w.photo_seq += 1;
            app.import(&format!("IMG_{:04}", w.photo_seq));
        }
        let v = w.commit(0, None, &mut app, "First upload")?.context("empty")?;
        w.move_ref(0, MAIN, &v)?;
        Ok(w)
    }

    fn project(&self, i: usize) -> Result<Project> {
        Project::load(&self.peers[i].store, &self.pid)
    }
    fn current_members(&self, i: usize) -> Result<Members> {
        wf::members(&self.peers[i].store, &self.project(i)?, &self.peers[i].members)
    }
    /// Valid tips this peer can use: ref valid, version present and verified.
    fn tips(&mut self, i: usize) -> Result<BTreeMap<String, (Id, wf::RefUpdate)>> {
        let project = self.project(i)?;
        let current = self.current_members(i)?;
        let p = &mut self.peers[i];
        let usable: BTreeSet<Id> = p
            .refs
            .iter()
            .copied()
            .filter(|r| wf::load_ref(&p.store, r).ok().and_then(|s| Id::parse(&s.update.body.version).ok()).is_some_and(|v| p.ok_versions.contains(&v)))
            .collect();
        Ok(wf::tips(&p.store, &project, &current, &usable).0)
    }
    fn forks(&mut self, i: usize) -> Result<Vec<Id>> {
        let project = self.project(i)?;
        let current = self.current_members(i)?;
        let p = &self.peers[i];
        let usable: BTreeSet<Id> = p.refs.iter().copied().filter(|r| wf::load_ref(&p.store, r).ok().and_then(|s| Id::parse(&s.update.body.version).ok()).is_some_and(|v| p.ok_versions.contains(&v))).collect();
        Ok(wf::tips(&p.store, &project, &current, &usable).1)
    }

    fn commit(&mut self, i: usize, head: Option<Id>, app: &mut App, msg: &str) -> Result<Option<Id>> {
        let project = self.project(i)?;
        let p = &mut self.peers[i];
        let mut repo = Repo { store: &p.store, project, head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        let v = repo.save(app, msg)?;
        // judge my own work by the same rule everyone else uses
        let Some(v) = v else { return Ok(None) };
        let members = self.current_members(i)?;
        let project = self.project(i)?;
        if wf::verify_version_v2(&self.peers[i].store, &project, &members, &v).is_err() {
            return Ok(None);
        }
        self.peers[i].ok_versions.insert(v);
        Ok(Some(v))
    }

    fn move_ref(&mut self, i: usize, name: &str, version: &Id) -> Result<Id> {
        let tips = self.tips(i)?;
        let prev = tips.get(name).map(|(id, u)| (*id, u.seq));
        let project = self.project(i)?;
        let p = &mut self.peers[i];
        let mut mt = wf::member_tips(&p.store, &p.members);
        let mut prev = prev;
        if p.behaviour == Behaviour::Cheat && self.rng.gen_bool(0.3) {
            // a smarter cheat: cite no members (backdate) and/or claim a huge seq
            mt.clear();
            if self.rng.gen_bool(0.5) {
                prev = prev.map(|(id, _)| (id, 1_000_000));
            }
            self.stats.cheats += 1;
        }
        let id = wf::update_ref(&p.store, &project, name, version, prev, &mt, &p.cert, &p.key)?;
        p.refs.insert(id);
        p.ok_versions.insert(*version);
        Ok(id)
    }

    fn my_head(&mut self, i: usize) -> Result<Option<Id>> {
        let tips = self.tips(i)?;
        let b = wf::branch_of(&self.peers[i].account);
        Ok(tips.get(&b).or(tips.get(MAIN)).and_then(|(_, u)| Id::parse(&u.version).ok()))
    }

    fn edit(&mut self, i: usize) -> Result<()> {
        let Some(head) = self.my_head(i)? else { return Ok(()) };
        let members = self.current_members(i)?;
        let acct = self.peers[i].account.clone();
        if !members.is_member(&acct) {
            return Ok(());
        }
        let mut app = App::checkout(&self.peers[i].store, &head)?;
        let photos = app.photos();
        if photos.is_empty() {
            return Ok(());
        }
        let ph = photos[self.rng.gen_range(0..photos.len())].clone();
        let (lane, field) = FIELDS[self.rng.gen_range(0..FIELDS.len())];
        let allowed = members.may_write(&acct, lane);
        let cheat = self.peers[i].behaviour == Behaviour::Cheat;
        if !allowed && !cheat {
            return Ok(());
        }
        let v: Value = match field {
            "comment" => json!(format!("note {}", self.rng.gen_range(0..1000))),
            "temperature" => json!(self.rng.gen_range(30..90) * 100),
            "rating" => json!(self.rng.gen_range(0..6)),
            _ => json!(self.rng.gen_range(-40..40)),
        };
        if let Some(it) = app.items.get_mut(&format!("{ph}#{lane}")) {
            it.state[field] = v;
        }
        if let Some(nv) = self.commit(i, Some(head), &mut app, "edit")? {
            self.move_ref(i, &wf::branch_of(&acct), &nv)?;
            if allowed && !cheat {
                self.stats.edits += 1;
                self.edits.push((acct, nv));
            } else {
                self.stats.cheats += 1;
            }
        }
        Ok(())
    }

    /// Bring main into my branch (only I write my branch, so this never forks).
    fn catch_up(&mut self, i: usize) -> Result<()> {
        let tips = self.tips(i)?;
        let b = wf::branch_of(&self.peers[i].account);
        let (Some((_, mu)), Some((_, bu))) = (tips.get(MAIN), tips.get(&b)) else { return Ok(()) };
        let (main, mine) = (Id::parse(&mu.version)?, Id::parse(&bu.version)?);
        if project::is_ancestor(&self.peers[i].store, &main, &mine)? {
            return Ok(());
        }
        let project = self.project(i)?;
        let p = &mut self.peers[i];
        let mut app = App::checkout(&p.store, &mine)?;
        let mut repo = Repo { store: &p.store, project, head: Some(mine), cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        repo.integrate_checked(&mut app, &main)?;
        let nv = repo.head.context("no head")?;
        let members = wf::members(&p.store, &Project::load(&p.store, &self.pid)?, &p.members)?;
        if wf::verify_version_v2(&p.store, &Project::load(&p.store, &self.pid)?, &members, &nv).is_err() {
            return Ok(());
        }
        p.ok_versions.insert(nv);
        if nv != mine {
            self.move_ref(i, &b, &nv)?;
            self.stats.catch_ups += 1;
        }
        Ok(())
    }

    /// An approver accepts one pending branch (or a forked-off approval) into main.
    fn approve_one(&mut self, i: usize) -> Result<bool> {
        let members = self.current_members(i)?;
        let acct = self.peers[i].account.clone();
        let cheat = self.peers[i].behaviour == Behaviour::Cheat;
        if !members.is_approver(&acct) && !cheat {
            return Ok(false);
        }
        let tips = self.tips(i)?;
        let Some((_, mu)) = tips.get(MAIN) else { return Ok(false) };
        let main = Id::parse(&mu.version)?;
        let mut pending: Vec<Id> = tips.iter().filter(|(n, _)| n.starts_with("branch/")).filter_map(|(_, (_, u))| Id::parse(&u.version).ok()).collect();
        let forks = self.forks(i)?;
        pending.extend(forks.iter().filter(|f| self.peers[i].ok_versions.contains(f)));
        let store = &self.peers[i].store;
        pending.retain(|v| !project::is_ancestor(store, v, &main).unwrap_or(true));
        if pending.is_empty() {
            return Ok(false);
        }
        let pick = pending[self.rng.gen_range(0..pending.len())];
        if forks.contains(&pick) {
            self.stats.forks_reoffered += 1;
        }
        let prefer = self.rng.gen_bool(0.5);
        let project = self.project(i)?;
        let p = &self.peers[i];
        let (nv, n) = wf::approve(&p.store, &project, &main, &pick, prefer, &policy, &p.cert, &p.key)?;
        self.move_ref(i, MAIN, &nv)?;
        if members.is_approver(&acct) {
            self.stats.approvals += 1;
            self.stats.clash_decisions += n as u64;
        } else {
            self.stats.cheats += 1;
        }
        Ok(true)
    }

    fn change_members(&mut self, i: usize) -> Result<()> {
        let members = self.current_members(i)?;
        if !members.is_admin(&self.peers[i].account) {
            return Ok(());
        }
        let others: Vec<String> = self.peers.iter().skip(1).map(|p| p.account.clone()).collect();
        let who = others[self.rng.gen_range(0..others.len())].clone();
        let op = match self.rng.gen_range(0..10) {
            0..=5 => MemberOp::Set { account: who, role: Role::Editor, lanes: vec![], approver: self.rng.gen_bool(0.5) },
            6..=8 => MemberOp::Remove { account: who },
            _ => {
                let dev = self.peers.iter().find(|p| p.account == who).map(|p| p.cert.body.device.clone()).unwrap_or_default();
                MemberOp::BlockDevice { device: dev }
            }
        };
        let project = self.project(i)?;
        let p = &mut self.peers[i];
        let tips = wf::member_tips(&p.store, &p.members);
        let id = wf::add_member_entry(&p.store, &project, tips, op, &p.cert, &p.key)?;
        p.members.insert(id);
        self.stats.member_changes += 1;
        // seal: re-sign main at the same version, citing the new members
        if let Some((_, mu)) = self.tips(i)?.get(MAIN).cloned() {
            let v = Id::parse(&mu.version)?;
            self.move_ref(i, MAIN, &v)?;
        }
        Ok(())
    }

    /// `to` learns everything `from` knows. A cut sync copies only part of it.
    fn sync(&mut self, from: usize, to: usize, cut: bool) -> Result<()> {
        self.stats.syncs += 1;
        let limit = if cut {
            self.stats.cut_syncs += 1;
            Some(self.rng.gen_range(0..30usize))
        } else {
            None
        };
        let mut moved = 0usize;
        let (fr, tr): (BTreeSet<Id>, BTreeSet<Id>) = (self.peers[from].refs.clone(), self.peers[from].members.clone());
        for id in tr.iter().chain(fr.iter()) {
            if limit.is_some_and(|l| moved >= l) {
                return Ok(());
            }
            let b = self.peers[from].store.get(id)?;
            self.peers[to].store.put_verified(id, &b)?;
            moved += 1;
        }
        // versions first (closure), then the receiver may know the refs
        for r in &fr {
            let s = wf::load_ref(&self.peers[from].store, r)?;
            let v = Id::parse(&s.update.body.version)?;
            if limit.is_some_and(|l| moved >= l) {
                return Ok(());
            }
            let (n, _) = copy_version_closure(&self.peers[from].store, &self.peers[to].store, &v)?;
            moved += n as usize;
        }
        self.peers[to].members.extend(tr);
        self.peers[to].refs.extend(fr.iter().copied());
        // verify new versions: main-reachable ones are vouched by the approver (signature and
        // certificate still checked); branch ones must satisfy the lanes of the current members.
        let project = self.project(to)?;
        let current = self.current_members(to)?;
        let p = &mut self.peers[to];
        // Judge each update's work by the members it cites (the same verdict on every device, whenever
        // it arrives), not by the members this device happens to hold right now (E17: 2/500 disagreed).
        let mut by_ref: Vec<(String, Id, Members)> = vec![];
        for r in &fr {
            if wf::check_ref(&p.store, &project, &current, r).is_err() {
                continue;
            }
            let s = wf::load_ref(&p.store, r)?;
            let cited: Vec<Id> = s.update.body.members.iter().filter_map(|m| Id::parse(m).ok()).collect();
            let at = wf::members(&p.store, &project, &wf::member_closure(&p.store, &cited))?;
            by_ref.push((s.update.body.name.clone(), Id::parse(&s.update.body.version)?, at));
        }
        for (name, v, at) in by_ref {
            let hist = match project::history(&p.store, &[v]) {
                Ok(h) => h,
                Err(e) => bail!("STUCK: {} cannot read history after sync: {e:#}", p.name),
            };
            let mut ok = true;
            for (vid, _) in hist.iter().rev() {
                if p.ok_versions.contains(vid) {
                    continue;
                }
                let res = if name == MAIN {
                    project::verify_version_by(&p.store, &project, vid, &|_, _| true).map(|_| ())
                } else {
                    wf::verify_version_v2(&p.store, &project, &at, vid).map(|_| ())
                };
                if res.is_err() {
                    ok = false;
                    break;
                }
            }
            if ok {
                for (vid, _) in hist {
                    p.ok_versions.insert(vid);
                }
            }
        }
        Ok(())
    }

    fn honest_current(&self) -> Result<Vec<usize>> {
        let m = self.current_members(0)?;
        Ok((0..self.peers.len()).filter(|&i| self.peers[i].behaviour == Behaviour::Honest && m.device_ok(&self.peers[i].cert)).collect())
    }

    fn gossip_all(&mut self, idx: &[usize]) -> Result<()> {
        for &a in idx {
            for &b in idx {
                if a != b {
                    self.sync(a, b, false)?;
                }
            }
        }
        Ok(())
    }
}

fn run_seed(seed: u64, ops: usize) -> Result<Stats> {
    let mut w = World::new(seed)?;
    let n = w.peers.len();
    for _ in 0..ops {
        let i = w.rng.gen_range(0..n);
        match w.rng.gen_range(0..20) {
            0..=8 => w.edit(i)?,
            9..=10 => w.catch_up(i)?,
            11..=12 => {
                w.approve_one(i)?;
            }
            13 => {
                if i == 0 || w.rng.gen_bool(0.3) {
                    w.change_members(0)?;
                }
            }
            _ => {
                let j = w.rng.gen_range(0..n);
                if i != j {
                    let cut = w.rng.gen_bool(0.15);
                    w.sync(i, j, cut)?;
                }
            }
        }
    }
    // final: everyone honest and current talks; the owner approves everything pending; repeat
    let mut rounds = 0;
    loop {
        rounds += 1;
        let hc = w.honest_current()?;
        w.gossip_all(&(0..n).collect::<Vec<_>>())?;
        for &i in &hc {
            w.catch_up(i)?;
        }
        w.gossip_all(&hc)?;
        while w.approve_one(0)? {
            w.gossip_all(&hc)?;
        }
        for &i in &hc {
            w.catch_up(i)?;
        }
        w.gossip_all(&hc)?;
        // W1: everyone honest and current sees the same tips
        let t0 = w.tips(hc[0])?;
        let mut same = true;
        for &i in &hc[1..] {
            if w.tips(i)?.iter().map(|(k, (id, _))| (k.clone(), *id)).collect::<Vec<_>>() != t0.iter().map(|(k, (id, _))| (k.clone(), *id)).collect::<Vec<_>>() {
                same = false;
            }
        }
        let main = Id::parse(&t0.get(MAIN).context("no main")?.1.version)?;
        let store = &w.peers[hc[0]].store;
        let all_in = t0.iter().filter(|(k, _)| k.starts_with("branch/")).all(|(_, (_, u))| Id::parse(&u.version).ok().is_some_and(|v| project::is_ancestor(store, &v, &main).unwrap_or(false)));
        if same && all_in {
            break;
        }
        if rounds > 8 {
            let mut diff = String::new();
            let base: BTreeMap<String, Id> = t0.iter().map(|(k, (id, _))| (k.clone(), *id)).collect();
            for &i in &hc[1..] {
                let ti: BTreeMap<String, Id> = w.tips(i)?.iter().map(|(k, (id, _))| (k.clone(), *id)).collect();
                for k in base.keys().chain(ti.keys()).collect::<BTreeSet<_>>() {
                    if base.get(k) != ti.get(k) {
                        diff.push_str(&format!(" [{} vs {}: {k} {:?} / {:?}; members {} vs {}]", w.peers[hc[0]].name, w.peers[i].name,
                            base.get(k).map(|x| x.short()), ti.get(k).map(|x| x.short()), w.peers[hc[0]].members.len(), w.peers[i].members.len()));
                    }
                }
            }
            bail!("W1 FAILED: no agreement after {rounds} rounds (same={same}, all approved={all_in}){diff}");
        }
    }
    let hc = w.honest_current()?;
    let t0 = w.tips(hc[0])?;
    let main = Id::parse(&t0[MAIN].1.version)?;
    let store = &w.peers[hc[0]].store;
    // W2: every edit by a still-current honest member is in main
    let m = w.current_members(hc[0])?;
    for (acct, v) in &w.edits {
        if !m.is_member(acct) {
            continue; // removed by the owner: their pending work is intentionally not accepted
        }
        if !project::is_ancestor(store, v, &main)? {
            let peer = w.peers.iter().find(|p| &p.account == acct).context("peer")?;
            let blocked = m.blocked.contains(&peer.cert.body.device);
            if blocked {
                continue; // a blocked device's work is rejected by design
            }
            let b = wf::branch_of(acct);
            let on_tip = t0.get(&b).and_then(|(_, u)| Id::parse(&u.version).ok()).map(|tip| project::is_ancestor(store, v, &tip).unwrap_or(false));
            // updates of this branch the owner knows: any that carries the edit but is not the tip (a fork)?
            let mut carriers = 0;
            for r in &w.peers[hc[0]].refs {
                if let Ok(s) = wf::load_ref(store, r) {
                    if s.update.body.name == b && Id::parse(&s.update.body.version).ok().is_some_and(|x| project::is_ancestor(store, v, &x).unwrap_or(false)) {
                        carriers += 1;
                    }
                }
            }
            bail!("W2 FAILED: an edit by {acct} is not in main (edit on branch tip: {on_tip:?}, branch updates carrying it: {carriers})");
        }
    }
    // W3: replay the canonical main chain and every branch tip; each step must have been allowed
    let project = Project::load(store, &w.pid)?;
    for (name, (id, _)) in &t0 {
        wf::check_ref(store, &project, &m, id).with_context(|| format!("W3 FAILED on {name}"))?;
    }
    Ok(w.stats)
}

#[test]
fn sim_workflow() -> Result<()> {
    let seeds: u64 = std::env::var("SIM_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let ops: usize = std::env::var("SIM_OPS").ok().and_then(|s| s.parse().ok()).unwrap_or(150);
    let start: u64 = std::env::var("SIM_START").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let (mut ok, mut fails) = (0, BTreeMap::<String, u64>::new());
    let mut examples = vec![];
    let mut total = Stats::default();
    for seed in start..start + seeds {
        match run_seed(seed, ops) {
            Ok(s) => {
                ok += 1;
                total.edits += s.edits;
                total.approvals += s.approvals;
                total.clash_decisions += s.clash_decisions;
                total.catch_ups += s.catch_ups;
                total.syncs += s.syncs;
                total.cut_syncs += s.cut_syncs;
                total.member_changes += s.member_changes;
                total.cheats += s.cheats;
                total.forks_reoffered += s.forks_reoffered;
            }
            Err(e) => {
                let m = format!("{e:#}");
                let kind = m.split(':').next().unwrap_or("?").to_string();
                *fails.entry(kind).or_default() += 1;
                if examples.len() < 4 {
                    examples.push(format!("seed {seed}: {}", m.chars().take(240).collect::<String>()));
                }
            }
        }
    }
    println!("{}", json!({"model": "v2 main+branches+approvals", "seeds": seeds, "ops_per_seed": ops, "passed": ok, "failures": fails, "stats": format!("{total:?}"), "examples": examples}));
    Ok(())
}
