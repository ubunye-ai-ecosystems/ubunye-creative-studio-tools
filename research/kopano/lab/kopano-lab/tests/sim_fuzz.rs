//! E17 stage 1: randomised reliability simulator. Many seeds; each seed is a team doing random
//! work (edits in permitted and forbidden lanes, imports, deletes, restores) and random syncs,
//! some of them cut off part way, some peers malicious. At the end everyone syncs fully and we
//! check four invariants:
//!   I1 convergence: every honest peer ends with the same content
//!   I2 no silent loss: every accepted edit is in the final state, or replaced by a later edit
//!      that saw it, or kept in a clash record
//!   I3 safety: every version anyone accepted passes the full permission check
//!   I4 liveness after interruptions: a sync cut off part way never leaves a peer stuck
//! Run: SIM_SEEDS=500 cargo test --release -p kopano-lab --test sim_fuzz -- --nocapture
//! The test fails if any seed diverges, sticks or loses an edit. To study one seed:
//! SIM_FIRST=14 SIM_SEEDS=1 SIM_DEBUG=1 (prints each final round and every refusal);
//! SIM_ROUNDS sets how many final gossip rounds are allowed (default 6).

use anyhow::{bail, Context, Result};
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::identity::{DeviceCert, DeviceKey, Provider};
use kopano_core::merge::CONFLICT_PREFIX;
use kopano_core::project::{self, AssetRef, Item, Member, Project, Role};
use kopano_core::repo::{copy_version_closure, Repo};
use kopano_core::store::{Id, MemStore, ObjectStore};
use kopano_core::tree;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const LANES: [(&str, &str); 3] = [("develop", "lab.develop"), ("selects", "lab.selects"), ("notes", "lab.notes")];
const FIELDS: [(&str, &str); 6] = [
    ("develop", "exposure"),
    ("develop", "contrast"),
    ("develop", "temperature"),
    ("selects", "rating"),
    ("selects", "flag"),
    ("notes", "comment"),
];

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
                "selects" => json!({"rating": 0, "flag": "none"}),
                _ => json!({"comment": ""}),
            };
            let assets = if lane == "develop" { vec![AssetRef { hash: h.clone(), size: 1, role: "original".into() }] } else { vec![] };
            self.items.insert(format!("{photo}#{lane}"), Item { kind: kind.into(), lane: lane.into(), schema: 1, state, assets });
        }
    }
    fn photos(&self) -> Vec<String> {
        self.items.keys().filter_map(|k| k.split('#').next()).map(String::from).collect::<BTreeSet<_>>().into_iter().collect()
    }
}
impl Connector for App {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            app: "lab.sim".into(),
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

struct Peer {
    name: String,
    role: Role,
    lanes: Vec<String>,
    honest: bool,
    store: MemStore,
    key: DeviceKey,
    cert: DeviceCert,
    head: Option<Id>,
    app: App,
}

/// An accepted edit to check against the final state (I2).
struct EditLog {
    version: Id,
    key: String,
    field: String,
    value: Value,
}

#[derive(Default, Debug)]
struct Stats {
    edits: u64,
    forbidden_tries: u64,
    refused: u64,
    syncs: u64,
    cut_syncs: u64,
    stuck_after_cut: u64,
    merges: u64,
    clashes: u64,
    restores: u64,
    imports: u64,
    deletes: u64,
    objects_moved: u64,
    bytes_moved: u64,
    /// final gossip rounds this seed needed to converge
    rounds: u64,
}

struct World {
    pid: Id,
    peers: Vec<Peer>,
    log: Vec<EditLog>,
    rng: StdRng,
    stats: Stats,
    photo_seq: u32,
    /// use the original copier (true) or a cut-safe one (false)
    original_copier: bool,
}

fn may_write(role: Role, lanes: &[String], lane: &str) -> bool {
    match role {
        Role::Admin => true,
        Role::Editor => lanes.is_empty() || lanes.iter().any(|l| l == lane),
        Role::Commenter => lane == "notes",
        Role::Viewer => false,
    }
}

impl World {
    fn new(seed: u64, original_copier: bool) -> Result<World> {
        let mut rng = StdRng::seed_from_u64(seed);
        let provider = Provider::generate();
        // owner + 2..5 others with mixed roles; sometimes one dishonest peer (a viewer who edits anyway)
        let n = rng.gen_range(2..=5);
        let mut spec: Vec<(String, Role, Vec<String>, bool)> = vec![("owner".into(), Role::Admin, vec![], true)];
        for i in 0..n {
            let (role, lanes): (Role, Vec<String>) = match rng.gen_range(0..5) {
                0 | 1 => (Role::Editor, vec![]),
                2 => (Role::Editor, vec!["develop".into()]),
                3 => (Role::Commenter, vec![]),
                _ => (Role::Viewer, vec![]),
            };
            let honest = !(role == Role::Viewer && rng.gen_bool(0.5));
            spec.push((format!("p{i}"), role, lanes, honest));
        }
        let mut peers = Vec::new();
        let mut members = BTreeMap::new();
        for (i, (name, role, lanes, honest)) in spec.into_iter().enumerate() {
            let key = DeviceKey::generate();
            let cert = provider.issue(&format!("acct-{name}"), &name, &key, "device", 86_400_000)?;
            if i > 0 {
                members.insert(cert.body.account.clone(), Member { role, lanes: lanes.clone() });
            }
            peers.push(Peer { name, role, lanes, honest, store: MemStore::default(), key, cert, head: None, app: App::default() });
        }
        let p = Project::create(&peers[0].store, "sim", vec![provider.public_hex()], &peers[0].cert, &peers[0].key, members)?;
        let g = peers[0].store.get(&p.id)?;
        for q in &peers {
            q.store.put_verified(&p.id, &g)?;
            q.store.flush()?;
        }
        let mut w = World { pid: p.id, peers, log: vec![], rng, stats: Stats::default(), photo_seq: 0, original_copier };
        for _ in 0..w.rng.gen_range(3..8) {
            w.photo_seq += 1;
            let ph = format!("IMG_{:04}", w.photo_seq);
            w.peers[0].app.import(&ph);
        }
        w.save(0, "import")?;
        Ok(w)
    }

    fn repo(&mut self, i: usize) -> Result<Repo<'_>> {
        let p = &self.peers[i];
        Ok(Repo { store: &p.store, project: Project::load(&p.store, &self.pid)?, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) })
    }

    fn save(&mut self, i: usize, msg: &str) -> Result<Option<Id>> {
        let pid = self.pid;
        let p = &mut self.peers[i];
        let mut repo = Repo { store: &p.store, project: Project::load(&p.store, &pid)?, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        let v = repo.save(&mut p.app, msg)?;
        p.head = repo.head;
        Ok(v)
    }

    /// One random edit by peer i. Honest peers only edit lanes they may write; dishonest ones try anything.
    fn random_edit(&mut self, i: usize) -> Result<()> {
        let photos = self.peers[i].app.photos();
        if photos.is_empty() {
            return Ok(());
        }
        let ph = photos[self.rng.gen_range(0..photos.len())].clone();
        let (lane, field) = FIELDS[self.rng.gen_range(0..FIELDS.len())];
        let allowed = may_write(self.peers[i].role, &self.peers[i].lanes, lane);
        if self.peers[i].honest && !allowed {
            return Ok(());
        }
        let value = match field {
            "flag" => json!(["none", "pick", "reject"][self.rng.gen_range(0..3)]),
            "comment" => json!(format!("note {}", self.rng.gen_range(0..1000))),
            "temperature" => json!(self.rng.gen_range(30..90) * 100),
            "rating" => json!(self.rng.gen_range(0..6)),
            _ => json!(self.rng.gen_range(-40..40)),
        };
        let key = format!("{ph}#{lane}");
        let Some(it) = self.peers[i].app.items.get_mut(&key) else { return Ok(()) };
        it.state[field] = value.clone();
        if let Some(v) = self.save(i, "edit")? {
            if allowed {
                self.stats.edits += 1;
                self.log.push(EditLog { version: v, key, field: field.into(), value });
            } else {
                self.stats.forbidden_tries += 1;
            }
        }
        Ok(())
    }

    fn random_structural(&mut self, i: usize) -> Result<()> {
        let full = matches!(self.peers[i].role, Role::Admin) || (self.peers[i].role == Role::Editor && self.peers[i].lanes.is_empty());
        if !full || !self.peers[i].honest {
            return Ok(());
        }
        match self.rng.gen_range(0..3) {
            0 => {
                self.photo_seq += 1;
                let ph = format!("IMG_{:04}", self.photo_seq);
                self.peers[i].app.import(&ph);
                if self.save(i, "import")?.is_some() {
                    self.stats.imports += 1;
                }
            }
            1 => {
                let photos = self.peers[i].app.photos();
                if photos.len() > 2 {
                    let ph = photos[self.rng.gen_range(0..photos.len())].clone();
                    self.peers[i].app.items.retain(|k, _| !k.starts_with(&format!("{ph}#")));
                    if self.save(i, "delete")?.is_some() {
                        self.stats.deletes += 1;
                    }
                }
            }
            _ => {
                let Some(h) = self.peers[i].head else { return Ok(()) };
                let hist = project::history(&self.peers[i].store, &[h])?;
                if hist.len() > 2 {
                    let target = hist[self.rng.gen_range(1..hist.len())].0;
                    let pid = self.pid;
                    let p = &mut self.peers[i];
                    let mut repo = Repo { store: &p.store, project: Project::load(&p.store, &pid)?, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
                    repo.restore(&mut p.app, &target)?;
                    p.head = repo.head;
                    self.stats.restores += 1;
                }
            }
        }
        Ok(())
    }

    /// Copy that stops after `limit` objects (a transfer cut off part way), same order as the copier under test.
    fn cut_copy(from: &dyn ObjectStore, to: &dyn ObjectStore, head: &Id, limit: usize, original: bool) -> Result<()> {
        let mut n = 0;
        if original {
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
                        if n >= limit {
                            return Ok(());
                        }
                        to.put_verified(&id, &from.get(&id)?)?;
                        n += 1;
                    }
                }
                stack.extend(ver.parents()?);
                if n >= limit {
                    return Ok(());
                }
                to.put_verified(&v, &from.get(&v)?)?;
                n += 1;
            }
            Ok(())
        } else {
            safe_copy(from, to, head, Some(limit)).map(|_| ())
        }
    }

    /// `to` pulls from `from`. Returns Ok(false) when refused (verification failed).
    fn sync(&mut self, from: usize, to: usize, cut: bool) -> Result<bool> {
        let Some(head) = self.peers[from].head else { return Ok(true) };
        self.stats.syncs += 1;
        if cut {
            self.stats.cut_syncs += 1;
            let limit = self.rng.gen_range(0..40);
            Self::cut_copy(&self.peers[from].store, &self.peers[to].store, &head, limit, self.original_copier)?;
            // the receiver does NOT integrate after a cut transfer; it tries again later
            return Ok(true);
        }
        let known: BTreeSet<Id> = match self.peers[to].head {
            Some(h) => project::history(&self.peers[to].store, &[h]).unwrap_or_default().into_iter().map(|(i, _)| i).collect(),
            None => BTreeSet::new(),
        };
        let (n, b) = if self.original_copier {
            copy_version_closure(&self.peers[from].store, &self.peers[to].store, &head)?
        } else {
            safe_copy(&self.peers[from].store, &self.peers[to].store, &head, None)?
        };
        self.stats.objects_moved += n;
        self.stats.bytes_moved += b;
        // verify every new version parents-first, exactly as the network pull does
        let pid = self.pid;
        let p = &mut self.peers[to];
        let project = Project::load(&p.store, &pid)?;
        let hist = match project::history(&p.store, &[head]) {
            Ok(h) => h,
            Err(e) => bail!("STUCK: {} cannot read the history it received: {e:#}", p.name),
        };
        let mut new: Vec<(Id, project::Version)> = hist.into_iter().filter(|(i, _)| !known.contains(i)).collect();
        let ids: BTreeSet<Id> = new.iter().map(|(i, _)| *i).collect();
        let mut done = BTreeSet::new();
        while !new.is_empty() {
            let before = new.len();
            let mut rest = vec![];
            for (id, v) in new {
                if v.parents()?.iter().all(|q| !ids.contains(q) || done.contains(q)) {
                    if let Err(e) = project::verify_version(&p.store, &project, &id) {
                        if std::env::var("SIM_DEBUG").is_ok() {
                            eprintln!("  {} refused {} from {}: {e:#}", p.name, id.short(), head.short());
                        }
                        self.stats.refused += 1;
                        return Ok(false);
                    }
                    done.insert(id);
                } else {
                    rest.push((id, v));
                }
            }
            if rest.len() == before {
                bail!("history cycle");
            }
            new = rest;
        }
        let mut repo = Repo { store: &p.store, project, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        match repo.integrate(&mut p.app, &head) {
            Ok(kopano_core::repo::Integrated::Merged { conflicts, .. }) => {
                self.stats.merges += 1;
                self.stats.clashes += conflicts.len() as u64;
            }
            Ok(kopano_core::repo::Integrated::AlreadyHad) => {
                if std::env::var("SIM_DEBUG").is_ok() && p.head != Some(head) {
                    eprintln!("  {} already had {} (head {:?})", p.name, head.short(), p.head.map(|h| h.short()));
                }
            }
            Ok(_) => {}
            Err(e) => bail!("STUCK: {} could not integrate: {e:#}", p.name),
        }
        p.head = repo.head;
        Ok(true)
    }

    fn honest(&self) -> Vec<usize> {
        (0..self.peers.len()).filter(|&i| self.peers[i].honest).collect()
    }

    fn content(&self, i: usize) -> Option<Id> {
        self.peers[i].head.and_then(|h| project::load_version(&self.peers[i].store, &h).ok()?.tree().ok()?)
    }
}

/// Copy whose invariant is "a stored version always has its whole closure": parents first, the
/// version object last, so a cut leaves nothing that later looks complete but is not.
fn safe_copy(from: &dyn ObjectStore, to: &dyn ObjectStore, head: &Id, limit: Option<usize>) -> Result<(u64, u64)> {
    let (mut n, mut bytes) = (0u64, 0u64);
    // post-order over the version graph
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
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
                if limit.is_some_and(|l| n as usize >= l) {
                    return Ok((n, bytes));
                }
                let b = from.get(&id)?;
                to.put_verified(&id, &b)?;
                n += 1;
                bytes += b.len() as u64;
            }
        }
    }
    Ok((n, bytes))
}

/// I2: every accepted edit is in the final state, superseded by a later version that saw it, or in a clash record.
fn check_no_silent_loss(w: &World, at: usize) -> Result<Vec<String>> {
    let p = &w.peers[at];
    let head = p.head.context("no head")?;
    let root = project::load_version(&p.store, &head)?.tree()?;
    let entries = tree::entries(&p.store, root)?;
    let hist = project::history(&p.store, &[head])?;
    let in_final: BTreeSet<Id> = hist.iter().map(|(i, _)| *i).collect();
    // versions that changed each (key) relative to their first parent
    let mut touched: BTreeMap<String, Vec<Id>> = BTreeMap::new();
    for (vid, v) in &hist {
        let parent_tree = match v.parents()?.first() {
            Some(pp) => project::load_version(&p.store, pp)?.tree()?,
            None => None,
        };
        for (k, _, _) in tree::diff(&p.store, parent_tree, v.tree()?)? {
            touched.entry(k).or_default().push(*vid);
        }
    }
    let clash_values = |key: &str, field: &str| -> Result<Vec<Value>> {
        let mut out = vec![];
        for (k, id) in &entries {
            if !k.starts_with(CONFLICT_PREFIX) {
                continue;
            }
            let rec = Item::get(&p.store, id)?;
            if rec.state["key"] != json!(key) {
                continue;
            }
            for side in ["mine", "theirs"] {
                if let Some(h) = rec.state[side].as_str() {
                    if let Ok(it) = Item::get(&p.store, &Id::parse(h)?) {
                        out.push(it.state[field].clone());
                    }
                }
            }
        }
        Ok(out)
    };
    let mut lost = vec![];
    for e in &w.log {
        if !in_final.contains(&e.version) {
            continue; // never reached the honest set (e.g. made by a peer whose work was refused)
        }
        let now = entries.get(&e.key).map(|id| Item::get(&p.store, id)).transpose()?.map(|it| it.state[&e.field].clone());
        if now.as_ref() == Some(&e.value) {
            continue;
        }
        let superseded = touched.get(&e.key).into_iter().flatten().any(|v| *v != e.version && project::is_ancestor(&p.store, &e.version, v).unwrap_or(false));
        if superseded {
            continue;
        }
        // A-B-A: the edit put back an item exactly as it was before (same content, same id), and
        // a version that changed the key saw that very item. Three-way merging cannot tell such an
        // edit from "unchanged" and the person who changed the key saw this state, so it is not a
        // loss (seed 825: pick then none again, while someone deleted the photo flagged none).
        let edited = tree::get(&p.store, project::load_version(&p.store, &e.version)?.tree()?, &e.key)?;
        let mut seen_by_later = false;
        for v in touched.get(&e.key).into_iter().flatten() {
            // only a version made without knowing the edit (an older one saw an older state)
            if *v == e.version || project::is_ancestor(&p.store, v, &e.version)? {
                continue;
            }
            if let Some(fp) = project::load_version(&p.store, v)?.parents()?.first() {
                if edited.is_some() && tree::get(&p.store, project::load_version(&p.store, fp)?.tree()?, &e.key)? == edited {
                    seen_by_later = true;
                    break;
                }
            }
        }
        if seen_by_later {
            if std::env::var("SIM_DEBUG").is_ok() {
                eprintln!("A-B-A edit, not a loss: {} {}={} in {}", e.key, e.field, e.value, e.version.short());
            }
            continue;
        }
        if clash_values(&e.key, &e.field)?.contains(&e.value) {
            continue;
        }
        if std::env::var("SIM_DEBUG").is_ok() {
            eprintln!("LOST {} {}={} in {}", e.key, e.field, e.value, e.version.short());
            let mut hs = hist.clone();
            hs.reverse();
            for (vid, v) in &hs {
                let ps = v.parents()?;
                let val = |t: Option<Id>| -> String { tree::get(&p.store, t, &e.key).ok().flatten().map(|i| Item::get(&p.store, &i).map(|it| it.state[&e.field].to_string()).unwrap_or_default()).unwrap_or("-".into()) };
                let mine = val(v.tree()?);
                let pv: Vec<String> = ps.iter().map(|q| val(project::load_version(&p.store, q).ok().and_then(|x| x.tree().ok()).flatten())).collect();
                if pv.iter().any(|x| *x != mine) || ps.is_empty() {
                    let desc = project::is_ancestor(&p.store, &e.version, vid).unwrap_or(false);
                    eprintln!("  {} by {} '{}' parents {:?} vals {:?} -> {} (after edit: {desc})", vid.short(), v.cert.body.name, v.v.body.message, ps.iter().map(|q| q.short()).collect::<Vec<_>>(), pv, mine);
                    if ps.len() == 2 {
                        for b in project::merge_bases(&p.store, &ps[0], &ps[1], 8)? {
                            let bt = project::load_version(&p.store, &b)?.tree()?;
                            eprintln!("      base {} val {} anc_of_edit {} edit_anc_of_base {}", b.short(), val(bt), project::is_ancestor(&p.store, &b, &e.version)?, project::is_ancestor(&p.store, &e.version, &b)?);
                        }
                    }
                }
            }
        }
        lost.push(format!("{} {}={} (now {:?})", e.key, e.field, e.value, now));
    }
    Ok(lost)
}

fn run_seed(seed: u64, original_copier: bool, ops: usize) -> Result<(Stats, Vec<String>)> {
    let mut w = World::new(seed, original_copier)?;
    let np = w.peers.len();
    for _ in 0..ops {
        let i = w.rng.gen_range(0..np);
        match w.rng.gen_range(0..10) {
            0..=4 => w.random_edit(i)?,
            5 => w.random_structural(i)?,
            _ => {
                let j = w.rng.gen_range(0..np);
                if i != j {
                    let cut = w.rng.gen_bool(0.15);
                    w.sync(i, j, cut)?;
                }
            }
        }
    }
    // final: gossip among honest peers until all match (bounded)
    let honest = w.honest();
    let mut rounds = 0;
    loop {
        rounds += 1;
        for &a in &honest {
            for &b in &honest {
                if a != b {
                    w.sync(a, b, false)?;
                }
            }
        }
        let first = w.content(honest[0]);
        if std::env::var("SIM_DEBUG").is_ok() {
            let t: Vec<String> = honest.iter().map(|&i| w.content(i).map(|x| x.short()).unwrap_or_default()).collect();
            eprintln!("seed {seed} round {rounds}: {:?}", t);
        }
        if honest.iter().all(|&i| w.content(i) == first) {
            break;
        }
        let max_rounds: usize = std::env::var("SIM_ROUNDS").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
        if rounds > max_rounds {
            bail!("I1 FAILED: honest peers did not converge after {rounds} rounds");
        }
    }
    w.stats.rounds = rounds as u64;
    // I3: re-verify every version in the final history
    let p0 = honest[0];
    let project = Project::load(&w.peers[p0].store, &w.pid)?;
    if let Some(h) = w.peers[p0].head {
        for (vid, _) in project::history(&w.peers[p0].store, &[h])? {
            project::verify_version(&w.peers[p0].store, &project, &vid).with_context(|| "I3 FAILED: an accepted version breaks the rules")?;
        }
    }
    let lost = check_no_silent_loss(&w, p0)?;
    Ok((w.stats, lost))
}

#[test]
fn sim_reliability() -> Result<()> {
    let seeds: u64 = std::env::var("SIM_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let ops: usize = std::env::var("SIM_OPS").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    for original in [false] {
        let label = if original { "original copier" } else { "cut-safe copier" };
        let (mut ok, mut stuck, mut diverged, mut lost_seeds, mut lost_total) = (0, 0, 0, 0, 0);
        let mut total = Stats::default();
        let mut examples: Vec<String> = vec![];
        let mut max_rounds = 0u64;
        let first_seed: u64 = std::env::var("SIM_FIRST").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        for seed in first_seed..first_seed + seeds {
            match run_seed(seed, original, ops) {
                Ok((s, lost)) => {
                    ok += 1;
                    if !lost.is_empty() {
                        lost_seeds += 1;
                        lost_total += lost.len();
                        if examples.len() < 3 {
                            examples.push(format!("seed {seed}: {}", lost[0]));
                        }
                    }
                    total.edits += s.edits;
                    total.forbidden_tries += s.forbidden_tries;
                    total.refused += s.refused;
                    total.syncs += s.syncs;
                    total.cut_syncs += s.cut_syncs;
                    total.merges += s.merges;
                    total.clashes += s.clashes;
                    total.restores += s.restores;
                    total.imports += s.imports;
                    total.deletes += s.deletes;
                    total.objects_moved += s.objects_moved;
                    total.bytes_moved += s.bytes_moved;
                    max_rounds = max_rounds.max(s.rounds);
                }
                Err(e) => {
                    let m = format!("{e:#}");
                    if m.contains("STUCK") {
                        stuck += 1;
                    } else if m.contains("I1 FAILED") {
                        diverged += 1;
                    }
                    if examples.len() < 3 {
                        examples.push(format!("seed {seed}: {}", m.chars().take(220).collect::<String>()));
                    }
                }
            }
        }
        total.rounds = max_rounds;
        println!(
            "{}",
            json!({"copier": label, "seeds": seeds, "ops_per_seed": ops, "completed": ok, "stuck": stuck, "diverged": diverged, "max_final_rounds": max_rounds,
                   "seeds_with_silent_loss": lost_seeds, "silent_losses": lost_total, "stats": format!("{total:?}"), "examples": examples})
        );
        // Fail loudly: a seed that diverges, sticks or loses an edit is a bug, never a statistic.
        assert_eq!(diverged, 0, "I1: honest peers diverged: {examples:?}");
        assert_eq!(stuck, 0, "I4: a peer got stuck: {examples:?}");
        assert_eq!(lost_total, 0, "I2: silent losses: {examples:?}");
        assert_eq!(ok as u64, seeds, "seeds failed for other reasons: {examples:?}");
    }
    Ok(())
}
