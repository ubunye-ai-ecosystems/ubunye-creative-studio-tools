//! E14: collaboration scenarios, in process. Each person has their own object store, device key,
//! certificate and app; "sync" moves the objects a head needs and verifies every new version
//! (parents first), exactly as the network pull does, then integrates. Each test prints one JSON
//! line: {"scenario", "result": "pass" | "gap", "detail"}. A "gap" is behaviour we observed and
//! consider wrong or missing for a product; the test asserts the observed behaviour so the gap
//! stays visible until fixed.
//!
//! Run: cargo test -p kopano-lab --test collab_scenarios -- --nocapture --test-threads 1

use anyhow::{bail, Context, Result};
use kopano_core::connector::{Connector, ConnectorInfo, KindInfo, CONTRACT_VERSION};
use kopano_core::identity::{DeviceCert, DeviceKey, Provider};
use kopano_core::merge::CONFLICT_PREFIX;
use kopano_core::project::{self, AssetRef, Item, Member, Project, Role};
use kopano_core::repo::{copy_version_closure, Integrated, Repo};
use kopano_core::store::{Id, MemStore, ObjectStore};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

// ---------- a three-lane photo app, shaped like the Light connector ----------

#[derive(Clone, Default)]
struct App {
    items: BTreeMap<String, Item>,
}

const LANES: [(&str, &str); 3] = [("develop", "lab.develop"), ("selects", "lab.selects"), ("notes", "lab.notes")];

impl App {
    fn import(&mut self, photo: &str) {
        let h = blake3::hash(photo.as_bytes()).to_hex().to_string();
        let init = |lane: &str| match lane {
            "develop" => json!({"exposure": 0.0, "contrast": 0.0, "temperature": 5500.0}),
            "selects" => json!({"rating": 0, "flag": "none"}),
            _ => json!({"comments": []}),
        };
        for (lane, kind) in LANES {
            self.items.insert(
                format!("{photo}#{lane}"),
                Item {
                    kind: kind.into(),
                    lane: lane.into(),
                    schema: 1,
                    state: init(lane),
                    assets: if lane == "develop" { vec![AssetRef { hash: h.clone(), size: 50_000_000, role: "original".into() }] } else { vec![] },
                },
            );
        }
    }
    fn set(&mut self, photo: &str, lane: &str, field: &str, v: Value) -> Result<()> {
        let it = self.items.get_mut(&format!("{photo}#{lane}")).with_context(|| format!("no {photo}#{lane}"))?;
        it.state[field] = v;
        Ok(())
    }
    fn get(&self, photo: &str, lane: &str, field: &str) -> Value {
        self.items.get(&format!("{photo}#{lane}")).map(|i| i.state[field].clone()).unwrap_or(Value::Null)
    }
    fn remove(&mut self, photo: &str) {
        for (lane, _) in LANES {
            self.items.remove(&format!("{photo}#{lane}"));
        }
    }
    fn photos(&self) -> BTreeSet<String> {
        self.items.keys().filter_map(|k| k.split('#').next()).map(String::from).collect()
    }
}

impl Connector for App {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            app: "lab.three-lane".into(),
            contract: CONTRACT_VERSION,
            kinds: LANES.iter().map(|(l, k)| KindInfo { kind: (*k).into(), lane: (*l).into(), schema: 1, mergeable: true }).collect(),
        }
    }
    fn snapshot(&mut self) -> Result<BTreeMap<String, Item>> {
        Ok(self.items.clone())
    }
    fn apply(&mut self, key: &str, item: Option<&Item>) -> Result<()> {
        match item {
            Some(i) => {
                self.items.insert(key.into(), i.clone());
            }
            None => {
                self.items.remove(key);
            }
        }
        Ok(())
    }
}

// ---------- people ----------

struct Person {
    name: String,
    store: MemStore,
    key: DeviceKey,
    cert: DeviceCert,
    head: Option<Id>,
    app: App,
}

struct Team {
    project_id: Id,
    people: BTreeMap<String, Person>,
}

impl Team {
    /// `roster`: (name, role, lanes). The first entry is the owner (admin).
    fn new(roster: &[(&str, Role, &[&str])], photos: usize) -> Result<Team> {
        Self::with_ttl(roster, photos, 86_400_000)
    }
    fn with_ttl(roster: &[(&str, Role, &[&str])], photos: usize, ttl_ms: u64) -> Result<Team> {
        let provider = Provider::generate();
        let mut people = BTreeMap::new();
        let mut members = BTreeMap::new();
        let mut owner = None;
        for (i, (name, role, lanes)) in roster.iter().enumerate() {
            let key = DeviceKey::generate();
            let cert = provider.issue(&format!("acct-{name}"), name, &key, "laptop", ttl_ms)?;
            if i == 0 {
                owner = Some((cert.clone(), DeviceKey::from_bytes(key.to_bytes())));
            } else if !lanes.contains(&"__nonmember") {
                members.insert(cert.body.account.clone(), Member { role: *role, lanes: lanes.iter().map(|s| s.to_string()).collect() });
            }
            people.insert(name.to_string(), Person { name: name.to_string(), store: MemStore::default(), key, cert, head: None, app: App::default() });
        }
        let (oc, ok) = owner.context("no owner")?;
        let first = roster[0].0;
        let p = Project::create(&people[first].store, "Lab shoot", vec![provider.public_hex()], &oc, &ok, members)?;
        let g = people[first].store.get(&p.id)?;
        for per in people.values() {
            per.store.put_verified(&p.id, &g)?;
            per.store.flush()?;
        }
        let mut t = Team { project_id: p.id, people };
        let owner = t.people.get_mut(first).unwrap();
        for i in 0..photos {
            owner.app.import(&format!("IMG_{i:04}"));
        }
        t.save(first, "Imported the shoot")?;
        Ok(t)
    }

    fn save(&mut self, who: &str, msg: &str) -> Result<Option<Id>> {
        let pid = self.project_id;
        let p = self.people.get_mut(who).context("no such person")?;
        let mut repo = Repo { store: &p.store, project: Project::load(&p.store, &pid)?, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        let v = repo.save(&mut p.app, msg)?;
        p.head = repo.head;
        Ok(v)
    }

    fn edit(&mut self, who: &str, photo: &str, lane: &str, field: &str, v: Value) -> Result<Option<Id>> {
        self.people.get_mut(who).unwrap().app.set(photo, lane, field, v)?;
        self.save(who, &format!("{who}: {photo} {field}"))
    }

    /// `to` pulls from `from`: copy what is missing, verify every new version (parents first), integrate.
    fn sync(&mut self, from: &str, to: &str) -> Result<String> {
        let pid = self.project_id;
        let Some(head) = self.people[from].head else { return Ok("nothing".into()) };
        let known: BTreeSet<Id> = match self.people[to].head {
            Some(h) => project::history(&self.people[to].store, &[h])?.into_iter().map(|(i, _)| i).collect(),
            None => BTreeSet::new(),
        };
        copy_version_closure(&self.people[from].store, &self.people[to].store, &head)?;
        let p = self.people.get_mut(to).unwrap();
        let project = Project::load(&p.store, &pid)?;
        // verify new versions parents-first (topological order)
        let mut new: Vec<(Id, project::Version)> = project::history(&p.store, &[head])?.into_iter().filter(|(i, _)| !known.contains(i)).collect();
        let ids: BTreeSet<Id> = new.iter().map(|(i, _)| *i).collect();
        let mut done = BTreeSet::new();
        while !new.is_empty() {
            let before = new.len();
            let mut rest = Vec::new();
            for (id, v) in new {
                if v.parents()?.iter().all(|q| !ids.contains(q) || done.contains(q)) {
                    project::verify_version(&p.store, &project, &id).with_context(|| format!("{to} refused version by {}", v.author()))?;
                    done.insert(id);
                } else {
                    rest.push((id, v));
                }
            }
            if rest.len() == before {
                bail!("history has a cycle");
            }
            new = rest;
        }
        let mut repo = Repo { store: &p.store, project, head: p.head, cert: p.cert.clone(), key: DeviceKey::from_bytes(p.key.to_bytes()) };
        let r = repo.integrate(&mut p.app, &head)?;
        p.head = repo.head;
        Ok(match r {
            Integrated::AlreadyHad => "already had".into(),
            Integrated::FastForward { applied } => format!("fast-forward, {applied} items"),
            Integrated::Merged { applied, conflicts, .. } => format!("merged, {applied} items, {} clash(es)", conflicts.len()),
        })
    }

    fn app(&self, who: &str) -> &App {
        &self.people[who].app
    }
    fn clashes(&self, who: &str) -> Result<Vec<String>> {
        let p = &self.people[who];
        let mut out = vec![];
        if let Some(h) = p.head {
            let root = project::load_version(&p.store, &h)?.tree()?;
            for (k, _) in kopano_core::tree::entries(&p.store, root)? {
                if k.starts_with(CONFLICT_PREFIX) {
                    out.push(k);
                }
            }
        }
        Ok(out)
    }
    /// Content fingerprint of a person's current state (items only, not version ids).
    fn content(&self, who: &str) -> String {
        serde_json::to_string(&self.people[who].app.items.iter().map(|(k, i)| (k.clone(), i.state.clone())).collect::<BTreeMap<_, _>>()).unwrap()
    }
}

fn report(scenario: &str, result: &str, detail: Value) {
    println!("{}", json!({"scenario": scenario, "result": result, "detail": detail}));
}

const OWNER: (&str, Role, &[&str]) = ("alice", Role::Admin, &[]);
const EDITOR: (&str, Role, &[&str]) = ("bob", Role::Editor, &[]);
const EDITOR2: (&str, Role, &[&str]) = ("dan", Role::Editor, &[]);
const RETOUCH: (&str, Role, &[&str]) = ("eve", Role::Editor, &["develop"]);
const CLIENT: (&str, Role, &[&str]) = ("carl", Role::Commenter, &[]);
const VIEWER: (&str, Role, &[&str]) = ("vic", Role::Viewer, &[]);
const OUTSIDER: (&str, Role, &[&str]) = ("mal", Role::Viewer, &["__nonmember"]);

// ---------- A. basic flow ----------

#[test]
fn a1_join_and_receive() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 50)?;
    let r = t.sync("alice", "bob")?;
    assert_eq!(t.content("alice"), t.content("bob"));
    report("A1 a teammate joins and receives the whole project (no media)", "pass", json!({"photos": t.app("bob").photos().len(), "sync": r}));
    Ok(())
}

#[test]
fn a2_edit_reaches_teammate() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 5)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.7))?;
    let r = t.sync("alice", "bob")?;
    assert_eq!(t.app("bob").get("IMG_0001", "develop", "exposure"), json!(0.7));
    report("A2 an edit reaches a teammate", "pass", json!({"sync": r}));
    Ok(())
}

#[test]
fn a3_relay_through_teammate_while_author_offline() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR, EDITOR2], 5)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0002", "develop", "contrast", json!(15))?;
    t.sync("alice", "bob")?;
    // Alice goes offline; Dan gets Alice's work from Bob. It is still Alice's signed version.
    let r = t.sync("bob", "dan")?;
    assert_eq!(t.app("dan").get("IMG_0002", "develop", "contrast"), json!(15));
    report("A3 work passes on through a teammate while its author is offline (still verified as the author's)", "pass", json!({"sync": r}));
    Ok(())
}

// ---------- B. working at the same time ----------

#[test]
fn b1_different_photos() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 5)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0000", "develop", "exposure", json!(1.0))?;
    t.edit("bob", "IMG_0004", "develop", "exposure", json!(-1.0))?;
    t.sync("bob", "alice")?;
    t.sync("alice", "bob")?;
    assert_eq!(t.content("alice"), t.content("bob"));
    assert!(t.clashes("alice")?.is_empty());
    report("B1 two people edit different photos at the same time", "pass", json!({"clashes": 0, "same_result_both_sides": true}));
    Ok(())
}

#[test]
fn b2_same_photo_different_settings() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.5))?;
    t.edit("bob", "IMG_0001", "develop", "contrast", json!(20))?;
    t.sync("bob", "alice")?;
    t.sync("alice", "bob")?;
    let a = t.app("alice");
    assert_eq!((a.get("IMG_0001", "develop", "exposure"), a.get("IMG_0001", "develop", "contrast")), (json!(0.5), json!(20)));
    assert_eq!(t.content("alice"), t.content("bob"));
    report("B2 same photo, different settings: both changes kept", "pass", json!({"exposure": 0.5, "contrast": 20}));
    Ok(())
}

#[test]
fn b3_same_setting_clash() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.9))?;
    t.edit("bob", "IMG_0001", "develop", "exposure", json!(-0.4))?;
    let r = t.sync("bob", "alice")?;
    let clashes = t.clashes("alice")?;
    t.sync("alice", "bob")?;
    assert_eq!(clashes.len(), 1);
    assert_eq!(t.clashes("bob")?.len(), 1, "the clash record travels to everyone");
    report("B3 same setting, different values: a clash record keeps both, nothing lost", "pass",
        json!({"sync": r, "kept_on_alice": t.app("alice").get("IMG_0001", "develop", "exposure"), "clash": clashes}));
    Ok(())
}

#[test]
fn b4_resolving_a_clash() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.9))?;
    t.edit("bob", "IMG_0001", "develop", "exposure", json!(-0.4))?;
    t.sync("bob", "alice")?;
    // Alice picks a value. Can the clash record be cleared through the app?
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.2))?;
    t.sync("alice", "bob")?;
    let left = t.clashes("bob")?.len();
    // The app cannot remove a conflict record: save() deliberately leaves them alone.
    assert_eq!(left, 1);
    report("B4 resolving a clash", "gap", json!({
        "observed": "after Alice picks a value the clash record stays forever; no call removes it",
        "needed": "a resolve step (a version that deletes the record, allowed to whoever may write the photo's lane)"}));
    Ok(())
}

#[test]
fn b5_three_offline_then_any_order() -> Result<()> {
    // Alice, Bob and Dan each edit offline; then they sync in two different orders.
    let mut outcomes = vec![];
    for order in [["bob", "dan"], ["dan", "bob"]] {
        let mut t = Team::new(&[OWNER, EDITOR, EDITOR2], 4)?;
        t.sync("alice", "bob")?;
        t.sync("alice", "dan")?;
        t.edit("alice", "IMG_0000", "develop", "exposure", json!(0.3))?;
        t.edit("bob", "IMG_0001", "selects", "rating", json!(4))?;
        t.edit("dan", "IMG_0002", "develop", "temperature", json!(6100))?;
        t.edit("dan", "IMG_0000", "develop", "contrast", json!(-5))?;
        for who in order {
            t.sync(who, "alice")?;
        }
        for who in ["bob", "dan"] {
            t.sync("alice", who)?;
        }
        assert_eq!(t.content("alice"), t.content("bob"));
        assert_eq!(t.content("alice"), t.content("dan"));
        outcomes.push(t.content("alice"));
    }
    let same = outcomes[0] == outcomes[1];
    assert!(same);
    report("B5 three people offline, merged in different orders: everyone ends with the same result", "pass", json!({"orders_tried": 2, "identical": same}));
    Ok(())
}

#[test]
fn b6_delete_vs_edit() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    t.people.get_mut("alice").unwrap().app.remove("IMG_0002");
    t.save("alice", "Removed a reject")?;
    t.edit("bob", "IMG_0002", "develop", "exposure", json!(1.2))?;
    let r = t.sync("bob", "alice")?;
    let kept = t.app("alice").photos().contains("IMG_0002");
    let clashes = t.clashes("alice")?;
    report("B6 one person deletes a photo while another edits it", if !clashes.is_empty() { "pass" } else { "gap" },
        json!({"sync": r, "photo_still_on_alice": kept, "clash_records": clashes.len(),
               "note": "delete vs edit must never silently drop the edit"}));
    assert!(!clashes.is_empty() || kept, "the edit was silently lost");
    Ok(())
}

#[test]
fn b7_both_import_at_once() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 2)?;
    t.sync("alice", "bob")?;
    t.people.get_mut("alice").unwrap().app.import("CARD_A_0001");
    t.save("alice", "Card A")?;
    t.people.get_mut("bob").unwrap().app.import("CARD_B_0001");
    t.save("bob", "Card B")?;
    t.sync("bob", "alice")?;
    t.sync("alice", "bob")?;
    let photos = t.app("bob").photos();
    assert!(photos.contains("CARD_A_0001") && photos.contains("CARD_B_0001"));
    report("B7 two people import different cards at the same time", "pass", json!({"photos_after": photos.len()}));
    Ok(())
}

#[test]
fn b8_long_offline() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 2000)?;
    t.sync("alice", "bob")?;
    let t0 = std::time::Instant::now();
    for i in 0..300 {
        t.edit("alice", &format!("IMG_{:04}", i), "develop", "exposure", json!(0.1))?;
        t.edit("bob", &format!("IMG_{:04}", 1000 + i), "selects", "rating", json!(3))?;
    }
    let edit_s = t0.elapsed().as_secs_f64();
    let t1 = std::time::Instant::now();
    let r = t.sync("bob", "alice")?;
    t.sync("alice", "bob")?;
    let sync_s = t1.elapsed().as_secs_f64();
    assert_eq!(t.content("alice"), t.content("bob"));
    report("B8 a week offline: 300 versions each on 2,000 photos, then one merge", "pass",
        json!({"sync": r, "edit_s": (edit_s * 10.0).round() / 10.0, "merge_and_back_s": (sync_s * 100.0).round() / 100.0}));
    Ok(())
}

#[test]
fn b9_ten_people_random_gossip() -> Result<()> {
    use rand::{Rng, SeedableRng};
    let names: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
    let leaked: Vec<&'static str> = names.iter().map(|s| &*Box::leak(s.clone().into_boxed_str())).collect();
    let roster: Vec<(&str, Role, &[&str])> = leaked.iter().enumerate().map(|(i, n)| (*n, if i == 0 { Role::Admin } else { Role::Editor }, &[][..])).collect();
    let mut t = Team::new(&roster, 30)?;
    for n in &leaked[1..] {
        t.sync("p0", n)?;
    }
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let mut clashes = 0;
    for _round in 0..20 {
        for n in &leaked {
            let photo = format!("IMG_{:04}", rng.gen_range(0..30));
            let (lane, field, v) = match rng.gen_range(0..3) {
                0 => ("develop", "exposure", json!(rng.gen_range(-20..20) as f64 / 10.0)),
                1 => ("develop", "contrast", json!(rng.gen_range(-50..50))),
                _ => ("selects", "rating", json!(rng.gen_range(0..6))),
            };
            t.edit(n, &photo, lane, field, v)?;
        }
        for _ in 0..10 {
            let a = leaked[rng.gen_range(0..10)];
            let b = leaked[rng.gen_range(0..10)];
            if a != b {
                let r = t.sync(a, b)?;
                if r.contains("clash") && !r.contains(" 0 clash") {
                    clashes += 1;
                }
            }
        }
    }
    // Finally everyone talks to p0 and back.
    for n in &leaked[1..] {
        t.sync(n, "p0")?;
    }
    for n in &leaked[1..] {
        t.sync("p0", n)?;
    }
    let all_same = leaked.iter().all(|n| t.content(n) == t.content("p0"));
    assert!(all_same);
    report("B9 ten people, 200 random edits, random syncs between pairs", "pass",
        json!({"everyone_identical_at_the_end": all_same, "merges_with_clashes": clashes, "open_clash_records": t.clashes("p0")?.len()}));
    Ok(())
}

// ---------- C. roles and permissions ----------

fn refused(r: Result<String>) -> (bool, String) {
    match r {
        Ok(s) => (false, s),
        Err(e) => (true, format!("{e:#}")),
    }
}

#[test]
fn c1_viewer_cannot_edit() -> Result<()> {
    let mut t = Team::new(&[OWNER, VIEWER], 3)?;
    t.sync("alice", "vic")?;
    t.edit("vic", "IMG_0001", "develop", "exposure", json!(3.0))?;
    let (no, why) = refused(t.sync("vic", "alice"));
    assert!(no);
    assert_eq!(t.app("alice").get("IMG_0001", "develop", "exposure"), json!(0.0));
    report("C1 a viewer's edit is refused by everyone", "pass", json!({"refused": no, "why": why}));
    Ok(())
}

#[test]
fn c2_client_comments_only() -> Result<()> {
    let mut t = Team::new(&[OWNER, CLIENT], 3)?;
    t.sync("alice", "carl")?;
    t.edit("carl", "IMG_0000", "notes", "comments", json!(["Love this one, can it be warmer?"]))?;
    let ok = t.sync("carl", "alice")?;
    t.edit("carl", "IMG_0000", "develop", "temperature", json!(7000))?;
    let (no, why) = refused(t.sync("carl", "alice"));
    assert!(no);
    report("C2 a client may comment but not change the edit", "pass", json!({"comment": ok, "edit_refused": no, "why": why}));
    Ok(())
}

#[test]
fn c3_retoucher_limited_to_develop() -> Result<()> {
    let mut t = Team::new(&[OWNER, RETOUCH], 3)?;
    t.sync("alice", "eve")?;
    t.edit("eve", "IMG_0001", "develop", "exposure", json!(0.4))?;
    let ok = t.sync("eve", "alice")?;
    t.edit("eve", "IMG_0001", "selects", "rating", json!(5))?;
    let (no, why) = refused(t.sync("eve", "alice"));
    assert!(no);
    report("C3 a retoucher may change develop settings but not ratings", "pass", json!({"develop": ok, "rating_refused": no, "why": why}));
    Ok(())
}

#[test]
fn c4_outsider_refused() -> Result<()> {
    let mut t = Team::new(&[OWNER, OUTSIDER], 3)?;
    t.sync("alice", "mal")?;
    t.edit("mal", "IMG_0001", "develop", "exposure", json!(2.0))?;
    let (no, why) = refused(t.sync("mal", "alice"));
    assert!(no);
    report("C4 someone with a valid account who is not on the project is refused", "pass", json!({"refused": no, "why": why}));
    Ok(())
}

#[test]
fn c5_laundering_through_an_editor() -> Result<()> {
    // A viewer's change hidden under an editor's valid version on top of it.
    let mut t = Team::new(&[OWNER, EDITOR, VIEWER], 3)?;
    t.sync("alice", "vic")?;
    t.sync("alice", "bob")?;
    t.edit("vic", "IMG_0000", "selects", "rating", json!(1))?;
    // Bob (careless or tricked) takes Vic's head as his base without checking, then edits.
    let vic_head = t.people["vic"].head.unwrap();
    copy_version_closure(&t.people["vic"].store, &t.people["bob"].store, &vic_head)?;
    let vic_app = t.people["vic"].app.clone();
    let bob = t.people.get_mut("bob").unwrap();
    bob.head = Some(vic_head);
    bob.app = vic_app;
    t.edit("bob", "IMG_0001", "develop", "exposure", json!(0.5))?;
    let (no, why) = refused(t.sync("bob", "alice"));
    assert!(no);
    report("C5 a forbidden change hidden under a valid teammate's version is still refused", "pass", json!({"refused": no, "why": why}));
    Ok(())
}

#[test]
fn c6_forged_merge() -> Result<()> {
    // Eve (develop only) makes a "merge" whose result changes a rating that neither parent had.
    let mut t = Team::new(&[OWNER, EDITOR, RETOUCH], 3)?;
    t.sync("alice", "bob")?;
    t.sync("alice", "eve")?;
    t.edit("bob", "IMG_0002", "develop", "contrast", json!(9))?;
    t.edit("eve", "IMG_0001", "develop", "exposure", json!(0.3))?;
    t.sync("bob", "eve")?; // a real merge
    t.edit("eve", "IMG_0000", "selects", "rating", json!(5))?; // then sneaks a rating
    let (no, why) = refused(t.sync("eve", "alice"));
    assert!(no);
    report("C6 a merge cannot be used to smuggle in a change the author may not make", "pass", json!({"refused": no, "why": why}));
    Ok(())
}

#[test]
fn c7_tampered_in_transit() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.6))?;
    let head = t.people["alice"].head.unwrap();
    let mut bytes = t.people["alice"].store.get(&head)?;
    let i = bytes.len() / 2;
    bytes[i] ^= 1;
    let ok = t.people["bob"].store.put_verified(&head, &bytes).is_err();
    assert!(ok);
    report("C7 a version changed on the way (one bit) is rejected by its fingerprint", "pass", json!({"rejected": ok}));
    Ok(())
}

#[test]
fn c8_expired_certificate() -> Result<()> {
    let mut t = Team::with_ttl(&[OWNER, EDITOR], 3, 1500)?;
    t.sync("alice", "bob")?;
    std::thread::sleep(std::time::Duration::from_millis(1700));
    t.edit("bob", "IMG_0001", "develop", "exposure", json!(0.6))?;
    let (no, why) = refused(t.sync("bob", "alice"));
    assert!(no);
    report("C8 a device whose certificate expired can no longer contribute", "pass", json!({"refused": no, "why": why}));
    Ok(())
}

#[test]
fn c9_remove_member_or_lost_laptop() -> Result<()> {
    // Membership is fixed in the project's genesis: there is no way to remove Bob or block his
    // stolen laptop before its certificate expires.
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    t.edit("bob", "IMG_0001", "develop", "exposure", json!(-3.0))?; // the thief edits
    let r = t.sync("bob", "alice")?;
    report("C9 removing a member, or blocking a stolen laptop", "gap", json!({
        "observed": format!("accepted ({r}); nothing can remove Bob or revoke his device until the certificate expires"),
        "needed": "a signed membership log (add, remove, change role) and a device revocation list checked on every sync"}));
    Ok(())
}

#[test]
fn c10_add_member_later() -> Result<()> {
    let t = Team::new(&[OWNER, EDITOR], 3)?;
    let project = Project::load(&t.people["alice"].store, &t.project_id)?;
    let can = project.genesis.members.len();
    report("C10 adding someone to the project after it started", "gap", json!({
        "observed": format!("members are fixed at creation ({can} members); a new person needs a new project"),
        "needed": "the same membership log as C9"}));
    Ok(())
}

// ---------- D. history and recovery ----------

#[test]
fn d1_restore_keeps_history() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    let v1 = t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.5))?.unwrap();
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(2.5))?;
    let pid = t.project_id;
    let a = t.people.get_mut("alice").unwrap();
    let mut repo = Repo { store: &a.store, project: Project::load(&a.store, &pid)?, head: a.head, cert: a.cert.clone(), key: DeviceKey::from_bytes(a.key.to_bytes()) };
    repo.restore(&mut a.app, &v1)?;
    a.head = repo.head;
    let n = project::history(&a.store, &[a.head.unwrap()])?.len();
    assert_eq!(t.app("alice").get("IMG_0001", "develop", "exposure"), json!(0.5));
    t.sync("alice", "bob")?;
    assert_eq!(t.app("bob").get("IMG_0001", "develop", "exposure"), json!(0.5));
    report("D1 going back to an earlier version keeps the later one in history, and teammates follow", "pass", json!({"versions_in_history": n}));
    Ok(())
}

#[test]
fn d2_restore_while_teammate_edits() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 3)?;
    t.sync("alice", "bob")?;
    let v1 = t.edit("alice", "IMG_0001", "develop", "exposure", json!(0.5))?.unwrap();
    t.edit("alice", "IMG_0001", "develop", "exposure", json!(2.5))?;
    t.sync("alice", "bob")?;
    t.edit("bob", "IMG_0002", "develop", "contrast", json!(30))?;
    let pid = t.project_id;
    let a = t.people.get_mut("alice").unwrap();
    let mut repo = Repo { store: &a.store, project: Project::load(&a.store, &pid)?, head: a.head, cert: a.cert.clone(), key: DeviceKey::from_bytes(a.key.to_bytes()) };
    repo.restore(&mut a.app, &v1)?;
    a.head = repo.head;
    t.sync("bob", "alice")?;
    t.sync("alice", "bob")?;
    let ok = t.app("bob").get("IMG_0001", "develop", "exposure") == json!(0.5) && t.app("bob").get("IMG_0002", "develop", "contrast") == json!(30);
    assert!(ok);
    report("D2 one person goes back in time while another keeps working: both survive", "pass", json!({"restored_value_kept": true, "teammate_edit_kept": true}));
    Ok(())
}

#[test]
fn d3_device_wiped_resyncs() -> Result<()> {
    let mut t = Team::new(&[OWNER, EDITOR], 200)?;
    t.sync("alice", "bob")?;
    for i in 0..20 {
        t.edit("bob", &format!("IMG_{i:04}"), "develop", "exposure", json!(0.2))?;
    }
    t.sync("bob", "alice")?;
    // Bob's laptop dies; a fresh install (same account and device certificate), empty store.
    let before = t.content("alice");
    let pid = t.project_id;
    let g = t.people["alice"].store.get(&pid)?;
    let b = t.people.get_mut("bob").unwrap();
    b.store = MemStore::default();
    b.store.put_verified(&pid, &g)?;
    b.head = None;
    b.app = App::default();
    let r = t.sync("alice", "bob")?;
    assert_eq!(t.content("bob"), before);
    report("D3 a laptop is wiped; the person gets everything back from a teammate", "pass", json!({"sync": r}));
    Ok(())
}
