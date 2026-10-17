//! Experiment E5 / proof of concept: Kopano between two independently identified devices over
//! iroh 1.x. Each peer is its own OS process with its own data folder and device key.
//!
//! - Device identity: one ed25519 key per device = its iroh endpoint id = the key that signs its
//!   versions. A lab "identity provider" (stand-in for ThabangVision) signs device certificates.
//! - Sync protocol `kopano/sync/0` (our ~150 lines): mutual certificate check bound to the
//!   TLS-authenticated endpoint id, then "heads" + batched "get objects" by id. Every object is
//!   verified by hash on arrival and every version by signature + permission before it is used.
//! - Media: iroh-blobs 0.103 (BLAKE3/bao verified streaming, resumable). Only project members may
//!   fetch. Media moves only when a peer asks for it; versions never carry media bytes.
//!
//! Commands (all take --dir <peer folder>):
//!   setup   --root R --photos <file>... --video <file>   create provider, alice, bob, project
//!   serve   --dir D [--relay-only]                     run until killed; writes D/addr.json
//!   pull    --dir D --from <addr.json> [--relay-only]  fetch + verify + integrate the peer's head
//!   fetch-media --dir D --from <addr.json> --hash H [--relay-only]   get one original
//!   edit    --dir D --key K --field F --value V        change a setting in the toy app and save
//!   status  --dir D
//!   corrupt --dir D                                    damage one stored object (recovery test)
//!   verify  --dir D                                    re-hash every object, list damaged ones

use anyhow::{anyhow, bail, Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use kopano_core::connector::Connector;
use kopano_core::identity::{DeviceCert, DeviceKey, Provider};
use kopano_core::project::{self, AssetRef, Item, Member, Project, Role};
use kopano_core::repo::{Integrated, Repo};
use kopano_core::store::{Id, ObjectStore, RedbStore};
use kopano_core::tree;
use kopano_lab::ToyEditor;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const SYNC_ALPN: &[u8] = b"kopano/sync/0";
const MAX_FRAME: usize = 64 << 20;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}
fn args_after(args: &[String], name: &str) -> Vec<String> {
    let Some(i) = args.iter().position(|a| a == name) else { return vec![] };
    args[i + 1..].iter().take_while(|a| !a.starts_with("--")).cloned().collect()
}

struct Peer {
    dir: PathBuf,
    store: RedbStore,
    key: DeviceKey,
    cert: DeviceCert,
    project: Project,
}

impl Peer {
    fn open(dir: &Path) -> Result<Self> {
        let store = RedbStore::open(dir.join("kopano.redb"))?;
        let key_bytes: [u8; 32] = std::fs::read(dir.join("device.key"))?.try_into().map_err(|_| anyhow!("bad device key"))?;
        let key = DeviceKey::from_bytes(key_bytes);
        let cert: DeviceCert = serde_json::from_slice(&std::fs::read(dir.join("cert.json"))?)?;
        let pid = Id::parse(std::fs::read_to_string(dir.join("project"))?.trim())?;
        let project = Project::load(&store, &pid)?;
        Ok(Self { dir: dir.into(), store, key, cert, project })
    }
    fn head(&self) -> Option<Id> {
        std::fs::read_to_string(self.dir.join("head")).ok().and_then(|s| Id::parse(s.trim()).ok())
    }
    fn set_head(&self, id: Option<Id>) -> Result<()> {
        match id {
            Some(h) => kopano_core::store::write_atomic(&self.dir.join("head"), h.hex().as_bytes()),
            None => Ok(()),
        }
    }
    fn app(&self) -> Result<ToyEditor> {
        let photos: BTreeMap<String, Item> = serde_json::from_slice(&std::fs::read(self.dir.join("app.json"))?)?;
        Ok(ToyEditor { photos })
    }
    fn save_app(&self, app: &ToyEditor) -> Result<()> {
        kopano_core::store::write_atomic(&self.dir.join("app.json"), &serde_json::to_vec_pretty(&app.photos)?)
    }
    fn repo(&self) -> Result<Repo<'_>> {
        Ok(Repo {
            store: &self.store,
            project: Project::load(&self.store, &self.project.id)?,
            head: self.head(),
            cert: self.cert.clone(),
            key: DeviceKey::from_bytes(self.key.to_bytes()),
        })
    }
    /// Is this certificate a current project member's device?
    fn check_peer(&self, cert: &DeviceCert, remote_id: &[u8; 32]) -> Result<()> {
        cert.check(&self.project.genesis.providers, kopano_core::identity::now_ms())?;
        if cert.body.device != hex::encode(remote_id) {
            bail!("certificate is for another device than the one connecting");
        }
        if !self.project.genesis.members.contains_key(&cert.body.account) {
            bail!("{} is not a member of this project", cert.body.name);
        }
        Ok(())
    }
    fn members_devices_ok(&self, cert: &DeviceCert) -> bool {
        cert.check(&self.project.genesis.providers, kopano_core::identity::now_ms()).is_ok()
            && self.project.genesis.members.contains_key(&cert.body.account)
    }
    async fn endpoint(&self, relay_only: bool, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
        let sk = SecretKey::from_bytes(&self.key.to_bytes());
        let mut b = Endpoint::builder(presets::N0).secret_key(sk).alpns(alpns);
        if let Ok(u) = std::env::var("KOPANO_RELAY_URL") {
            // Our own relay instead of n0's public ones.
            let url: iroh::RelayUrl = u.parse().context("KOPANO_RELAY_URL")?;
            b = b.relay_mode(iroh::endpoint::RelayMode::Custom(iroh::RelayMap::from(url)));
        }
        if relay_only {
            // No direct UDP at all: every byte must go through the relay (worst-case NAT).
            b = b.clear_ip_transports();
        }
        Ok(b.bind().await?)
    }
    async fn blobs(&self) -> Result<iroh_blobs::store::fs::FsStore> {
        Ok(iroh_blobs::store::fs::FsStore::load(self.dir.join("media")).await?)
    }
}

// ---- framing ----------------------------------------------------------------------------------

async fn write_frame(s: &mut iroh::endpoint::SendStream, bytes: &[u8]) -> Result<()> {
    s.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    s.write_all(bytes).await?;
    Ok(())
}
async fn read_frame(r: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        bail!("frame of {n} bytes refused");
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

// ---- server side ------------------------------------------------------------------------------

#[derive(Clone)]
struct SyncHandler {
    peer: Arc<Peer>,
}
impl std::fmt::Debug for SyncHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncHandler")
    }
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(conn).await {
            eprintln!("[serve] connection ended: {e:#}");
        }
        Ok(())
    }
}

impl SyncHandler {
    async fn serve(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        // 1. Mutual identity: they show their certificate first, we check it, then show ours.
        let their: DeviceCert = serde_json::from_slice(&read_frame(&mut recv).await?)?;
        if let Err(e) = self.peer.check_peer(&their, &remote) {
            write_frame(&mut send, format!("refused: {e}").as_bytes()).await?;
            send.finish()?;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), send.stopped()).await;
            bail!("refused {}: {e}", their.body.name);
        }
        write_frame(&mut send, &serde_json::to_vec(&self.peer.cert)?).await?;
        eprintln!("[serve] {} ({}) connected", their.body.name, their.body.device_label);
        // 2. Requests until they close.
        loop {
            let req = match read_frame(&mut recv).await {
                Ok(r) => r,
                Err(_) => break,
            };
            let req: serde_json::Value = serde_json::from_slice(&req)?;
            match req.get("op").and_then(|o| o.as_str()) {
                Some("heads") => {
                    let heads: Vec<String> = self.peer.head().into_iter().map(|h| h.hex()).collect();
                    write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"heads": heads, "project": self.peer.project.id.hex()}))?).await?;
                }
                Some("get") => {
                    let ids = req.get("ids").and_then(|i| i.as_array()).cloned().unwrap_or_default();
                    for id in ids.iter().take(10_000) {
                        let bytes = id.as_str().and_then(|s| Id::parse(s).ok()).and_then(|id| self.peer.store.get(&id).ok()).unwrap_or_default();
                        write_frame(&mut send, &bytes).await?;
                    }
                }
                Some("closure") => {
                    // Everything the client lacks, in one response: for each version the client does
                    // not have, the version plus the tree delta against its first parent.
                    let have: Vec<Id> = req.get("have").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).filter(|h| self.peer.store.has(h)).collect();
                    let known: BTreeSet<Id> = project::history(&self.peer.store, &have)?.into_iter().map(|(id, _)| id).collect();
                    let mut out = Vec::new();
                    for (id, v) in project::history(&self.peer.store, &self.peer.head().into_iter().collect::<Vec<_>>())? {
                        if known.contains(&id) {
                            continue;
                        }
                        out.push(id);
                        let parent_tree = match v.parents()?.first() {
                            Some(p) => project::load_version(&self.peer.store, p)?.tree()?,
                            None => None,
                        };
                        tree::missing(&self.peer.store, v.tree()?, parent_tree, &mut out)?;
                    }
                    let mut seen = HashSet::new();
                    out.retain(|i| seen.insert(*i));
                    write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"n": out.len()}))?).await?;
                    for id in out {
                        write_frame(&mut send, &self.peer.store.get(&id)?).await?;
                    }
                }
                _ => bail!("unknown request"),
            }
        }
        Ok(())
    }
}

/// Only project members' devices may fetch media (iroh-blobs event hook).
fn media_gate(allowed: HashSet<[u8; 32]>) -> iroh_blobs::provider::events::EventSender {
    use iroh_blobs::provider::events::{AbortReason, ConnectMode, EventMask, EventSender, ProviderMessage};
    let mask = EventMask { connected: ConnectMode::Intercept, ..EventMask::DEFAULT };
    let (tx, mut rx) = EventSender::channel(32, mask);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let ProviderMessage::ClientConnected(msg) = msg {
                let ok = msg.endpoint_id.map(|id| allowed.contains(id.as_bytes())).unwrap_or(false);
                eprintln!("[media] client {} {}", msg.endpoint_id.map(|i| i.fmt_short().to_string()).unwrap_or_default(), if ok { "allowed" } else { "REFUSED" });
                msg.tx.send(if ok { Ok(()) } else { Err(AbortReason::Permission) }).await.ok();
            }
        }
    });
    tx
}

async fn serve(dir: &Path, relay_only: bool) -> Result<()> {
    let dir = &std::path::absolute(dir)?;
    let peer = Arc::new(Peer::open(dir)?);
    let blobs = peer.blobs().await?;
    // Devices allowed to fetch media: the lab knows them from the setup folder's certificates.
    let mut allowed = HashSet::new();
    for f in std::fs::read_dir(dir.parent().context("no parent")?)?.flatten() {
        if let Ok(b) = std::fs::read(f.path().join("cert.json")) {
            if let Ok(c) = serde_json::from_slice::<DeviceCert>(&b) {
                if peer.members_devices_ok(&c) {
                    if let Ok(k) = hex::decode(&c.body.device) {
                        if let Ok(k) = <[u8; 32]>::try_from(k.as_slice()) {
                            allowed.insert(k);
                        }
                    }
                }
            }
        }
    }
    let ep = peer.endpoint(relay_only, vec![SYNC_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()]).await?;
    let blobs_proto = iroh_blobs::BlobsProtocol::new(&blobs, Some(media_gate(allowed)));
    let router = Router::builder(ep).accept(SYNC_ALPN, SyncHandler { peer: peer.clone() }).accept(iroh_blobs::ALPN, blobs_proto).spawn();
    let t = Instant::now();
    router.endpoint().online().await;
    let addr = router.endpoint().addr();
    std::fs::write(dir.join("addr.json"), serde_json::to_vec(&addr)?)?;
    eprintln!("[serve] {} online in {} ms, addr written", peer.cert.body.name, t.elapsed().as_millis());
    tokio::signal::ctrl_c().await?;
    router.shutdown().await?;
    Ok(())
}

// ---- client side ------------------------------------------------------------------------------

async fn connect(peer: &Peer, from: &Path, relay_only: bool, alpn: &[u8]) -> Result<(Endpoint, Connection, u128, String)> {
    let mut addr: EndpointAddr = serde_json::from_slice(&std::fs::read(from)?)?;
    if relay_only {
        addr = EndpointAddr::from_parts(addr.id, addr.relay_urls().cloned().map(iroh::TransportAddr::Relay));
    }
    let ep = peer.endpoint(relay_only, vec![]).await?;
    let t = Instant::now();
    let conn = ep.connect(addr, alpn).await?;
    let ms = t.elapsed().as_millis();
    let path = conn.paths().iter().find(|p| p.is_selected()).map(|p| format!("{:?}", p.remote_addr())).unwrap_or_else(|| "unknown".into());
    Ok((ep, conn, ms, path))
}

async fn pull(dir: &Path, from: &Path, relay_only: bool, closure: bool) -> Result<()> {
    let peer = Peer::open(dir)?;
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, SYNC_ALPN).await?;
    let t_all = Instant::now();
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, &serde_json::to_vec(&peer.cert)?).await?;
    if closure {
        // Pipelined: certificate, heads and closure requests leave in one flight (one round trip).
        write_frame(&mut send, br#"{"op":"heads"}"#).await?;
        let have: Vec<String> = peer.head().into_iter().map(|h| h.hex()).collect();
        write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"op": "closure", "have": have}))?).await?;
    }
    let reply = read_frame(&mut recv).await?;
    let their: DeviceCert = serde_json::from_slice(&reply).map_err(|_| anyhow!("{}", String::from_utf8_lossy(&reply)))?;
    peer.check_peer(&their, conn.remote_id().as_bytes())?;
    if !closure {
        write_frame(&mut send, br#"{"op":"heads"}"#).await?;
    }
    let heads: serde_json::Value = serde_json::from_slice(&read_frame(&mut recv).await?)?;
    if heads.get("project").and_then(|p| p.as_str()) != Some(&peer.project.id.hex()) {
        bail!("peer is on another project");
    }
    let their_heads: Vec<Id> = heads.get("heads").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).collect();
    // Walk the graph level by level, asking only for what we lack. One round trip per level.
    let (mut objects, mut bytes, mut rounds) = (0u64, 0u64, 0u32);
    let mut new_versions = Vec::new();
    if closure {
        // The closure request was already sent with the certificate.
        rounds = 1;
        let hdr: serde_json::Value = serde_json::from_slice(&read_frame(&mut recv).await?)?;
        let n = hdr.get("n").and_then(|n| n.as_u64()).unwrap_or(0).min(10_000_000);
        for _ in 0..n {
            let b = read_frame(&mut recv).await?;
            let id = Id::of(&b);
            peer.store.put_verified(&id, &b)?;
            objects += 1;
            bytes += b.len() as u64;
            if b.first() == Some(&b'{') {
                if serde_json::from_slice::<project::Version>(&b).is_ok() {
                    new_versions.push(id);
                }
            }
        }
    }
    let mut frontier: Vec<Id> = if closure { vec![] } else { their_heads.iter().copied().filter(|h| !peer.store.has(h)).collect() };
    let mut seen = BTreeSet::new();
    while !frontier.is_empty() {
        rounds += 1;
        let mut next = Vec::new();
        for batch in frontier.chunks(5_000) {
        let req = serde_json::json!({"op": "get", "ids": batch.iter().map(Id::hex).collect::<Vec<_>>()});
        write_frame(&mut send, &serde_json::to_vec(&req)?).await?;
        for id in batch {
            let b = read_frame(&mut recv).await?;
            if b.is_empty() {
                bail!("peer does not have object {}", id.short());
            }
            peer.store.put_verified(id, &b)?;
            objects += 1;
            bytes += b.len() as u64;
            let mut kids = Vec::new();
            if b.first() == Some(&b'L') || b.first() == Some(&b'B') {
                kids = tree::children(&b)?;
            } else if let Ok(v) = serde_json::from_slice::<project::Version>(&b) {
                kids.extend(v.tree()?);
                kids.extend(v.parents()?);
                new_versions.push(*id);
            }
            for k in kids {
                if !peer.store.has(&k) && seen.insert(k) {
                    next.push(k);
                }
            }
        }
        }
        frontier = next;
    }
    peer.store.flush()?;
    send.finish()?;
    conn.close(0u32.into(), b"done");
    let transfer_ms = t_all.elapsed().as_millis();
    // Verify every new version, oldest first (a child's check needs its parents).
    let t = Instant::now();
    for id in new_versions.iter().rev() {
        project::verify_version(&peer.store, &peer.project, id).with_context(|| format!("version {} refused", id.short()))?;
    }
    let verify_ms = t.elapsed().as_millis();
    let mut app = peer.app()?;
    let mut repo = peer.repo()?;
    let mut summary = String::from("nothing new");
    for h in &their_heads {
        summary = match repo.integrate(&mut app, h)? {
            Integrated::AlreadyHad => "already up to date".into(),
            Integrated::FastForward { applied } => format!("fast-forward, {applied} item(s) applied"),
            Integrated::Merged { version, applied, conflicts } => format!(
                "merged as {}, {applied} item(s) applied, {} clash(es): {}",
                version.short(),
                conflicts.len(),
                conflicts.iter().map(|c| format!("{} [{}]", c.key, c.fields.join(", "))).collect::<Vec<_>>().join("; ")
            ),
        };
    }
    peer.save_app(&app)?;
    peer.set_head(repo.head)?;
    ep.close().await;
    println!(
        "{{\"event\":\"pull\",\"path\":{:?},\"connect_ms\":{connect_ms},\"transfer_ms\":{transfer_ms},\"verify_ms\":{verify_ms},\"objects\":{objects},\"bytes\":{bytes},\"rounds\":{rounds},\"media_bytes\":0,\"result\":{:?}}}",
        path, summary
    );
    Ok(())
}

async fn fetch_media(dir: &Path, from: &Path, hash: &str, relay_only: bool) -> Result<()> {
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let before = blobs.remote().local(h).await?;
    let had = before.local_bytes();
    if before.is_complete() {
        println!("{{\"event\":\"media\",\"hash\":\"{}\",\"already_complete\":true,\"bytes_received\":0}}", &hash[..8]);
        return Ok(());
    }
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, iroh_blobs::ALPN).await?;
    let t = Instant::now();
    let stats = blobs.remote().fetch(conn.clone(), h).complete().await?;
    let ms = t.elapsed().as_millis();
    let after = blobs.remote().local(h).await?;
    conn.close(0u32.into(), b"done");
    ep.close().await;
    println!(
        "{{\"event\":\"media\",\"hash\":\"{}\",\"path\":{:?},\"connect_ms\":{connect_ms},\"had_before\":{had},\"bytes_received\":{},\"total_read\":{},\"ms\":{ms},\"mbit_s\":{:.0},\"complete\":{}}}",
        &hash[..8],
        path,
        stats.payload_bytes_read,
        stats.total_bytes_read(),
        stats.mbits(),
        after.is_complete()
    );
    Ok(())
}

// ---- setup and local commands -----------------------------------------------------------------

async fn setup(root: &Path, photos: &[String], video: Option<String>, synthetic: usize, extra_peers: usize) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let provider = Provider::generate();
    std::fs::write(root.join("provider.pub"), provider.public_hex())?;
    let (ka, kb, kc) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
    let ca = provider.issue("acct-alice", "Alice", &ka, "studio-desktop", 86_400_000)?;
    let cb = provider.issue("acct-bob", "Bob", &kb, "laptop", 86_400_000)?;
    // Carol has a valid account and device, but is not a member of the project.
    let cc = provider.issue("acct-carol", "Carol", &kc, "laptop", 86_400_000)?;
    let mut members = BTreeMap::new();
    members.insert("acct-bob".to_string(), Member { role: Role::Editor, lanes: vec!["develop".into()] });
    // Hash originals once (as an importer would) and build the toy app's items.
    let mut originals = Vec::new();
    for p in photos.iter().chain(video.iter()) {
        let mut f = std::fs::File::open(p)?;
        let mut h = blake3::Hasher::new();
        let n = std::io::copy(&mut f, &mut h)?;
        let key = Path::new(p).file_name().and_then(|s| s.to_str()).unwrap_or("x").to_string();
        originals.push((key, h.finalize().to_hex().to_string(), n, p.clone()));
    }
    // Extra team members (scale test): editors of the develop lane, each with their own device.
    let mut extra = Vec::new();
    for i in 0..extra_peers {
        let k = DeviceKey::generate();
        let c = provider.issue(&format!("acct-m{i}"), &format!("Member{i}"), &k, "laptop", 86_400_000)?;
        members.insert(c.body.account.clone(), Member { role: Role::Editor, lanes: vec!["develop".into()] });
        extra.push((format!("m{i}"), k, c));
    }
    let mut people: Vec<(String, &DeviceKey, &DeviceCert)> = vec![("alice".into(), &ka, &ca), ("bob".into(), &kb, &cb), ("carol".into(), &kc, &cc)];
    for (n, k, c) in &extra {
        people.push((n.clone(), k, c));
    }
    for (name, key, cert) in people.iter().map(|(n, k, c)| (n.as_str(), *k, *c)) {
        let d = root.join(name);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("device.key"), key.to_bytes())?;
        std::fs::write(d.join("cert.json"), serde_json::to_vec_pretty(cert)?)?;
    }
    let sa = RedbStore::open(root.join("alice/kopano.redb"))?;
    let project = Project::create(&sa, "Wedding shoot (lab)", vec![provider.public_hex()], &ca, &ka, members)?;
    for name in people.iter().map(|p| p.0.clone()) {
        let d = root.join(&name);
        if name != "alice" {
            let s = RedbStore::open(d.join("kopano.redb"))?;
            s.put_verified(&project.id, &sa.get(&project.id)?)?;
            s.flush()?;
        }
        std::fs::write(d.join("project"), project.id.hex())?;
    }
    // Alice's app: every original. Bob's app: the photos (he got them from the card too) but not
    // the video. Media stores: Alice holds everything; Bob holds the photos only.
    let mut list: Vec<(String, String, u64)> = originals.iter().map(|(k, h, n, _)| (k.clone(), h.clone(), *n)).collect();
    // Synthetic catalogue: items whose originals exist only as hashes (no media), for scale tests.
    for i in 0..synthetic {
        list.push((format!("IMG_{i:07}.ARW"), blake3::hash(format!("synthetic {i}").as_bytes()).to_hex().to_string(), 50_000_000));
    }
    let alice_app = ToyEditor::new(&list);
    std::fs::write(root.join("alice/app.json"), serde_json::to_vec(&alice_app.photos)?)?;
    for name in people.iter().map(|p| p.0.clone()).filter(|n| n != "alice") {
        std::fs::write(root.join(&name).join("app.json"), b"{}")?;
    }
    let ba = iroh_blobs::store::fs::FsStore::load(root.join("alice/media")).await?;
    let bb = iroh_blobs::store::fs::FsStore::load(root.join("bob/media")).await?;
    for (i, (_, h, _, p)) in originals.iter().enumerate() {
        let abs = std::path::absolute(p)?;
        let tag = ba.blobs().add_path(abs.clone()).await?;
        if tag.hash.to_hex() != *h {
            bail!("iroh-blobs and kopano disagree on the hash of {p}");
        }
        if i < photos.len() {
            bb.blobs().add_path(abs).await?;
        }
    }
    ba.shutdown().await?;
    bb.shutdown().await?;
    drop(sa);
    // Alice's first save.
    let alice = Peer::open(&root.join("alice"))?;
    let mut app = alice.app()?;
    let mut repo = alice.repo()?;
    let v = repo.save(&mut app, "Imported the shoot")?;
    alice.set_head(repo.head)?;
    println!("{{\"event\":\"setup\",\"project\":\"{}\",\"first_version\":\"{}\",\"originals\":{}}}", project.id.short(), v.map(|v| v.short()).unwrap_or_default(), originals.len());
    for (k, h, n, _) in &originals {
        println!("  {k}: {}… {} MB", &h[..12], n / 1_000_000);
    }
    Ok(())
}

fn edit(dir: &Path, key: &str, field: &str, value: f64, message: &str) -> Result<()> {
    let peer = Peer::open(dir)?;
    let mut app = peer.app()?;
    let item = app.photos.get_mut(key).with_context(|| format!("no item {key}"))?;
    let ptr = format!("/{}", field.replace('.', "/"));
    *item.state.pointer_mut(&ptr).with_context(|| format!("no setting {field}"))? = serde_json::json!(value);
    let mut repo = peer.repo()?;
    let t = Instant::now();
    let v = repo.save(&mut app, message)?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    peer.save_app(&app)?;
    peer.set_head(repo.head)?;
    println!("{{\"event\":\"edit\",\"who\":\"{}\",\"key\":\"{key}\",\"field\":\"{field}\",\"value\":{value},\"version\":\"{}\",\"save_ms\":{ms:.1}}}", peer.cert.body.name, v.map(|v| v.short()).unwrap_or_default());
    Ok(())
}

fn status(dir: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let mut app = peer.app()?;
    let repo = peer.repo()?;
    println!("{} on {} — head {}", peer.cert.body.name, peer.project.genesis.name, repo.head.map(|h| h.short()).unwrap_or_else(|| "none".into()));
    if let Some(h) = repo.head {
        for (id, v) in project::history(&peer.store, &[h])? {
            println!("  {}  {:<6} {:<45} lanes={:?}", id.short(), v.author(), v.v.body.message, v.v.body.lanes);
        }
    }
    for (k, item) in app.snapshot()? {
        let d = &item.state["develop"];
        println!("  item {k}: exposure={} contrast={} rating={}", d["exposure"], d["contrast"], item.state["rating"]);
    }
    for (k, c) in repo.conflicts()? {
        println!("  CLASH {k}: fields={} mine={} theirs={}", c.state["fields"], &c.state["mine"].as_str().unwrap_or("")[..8.min(c.state["mine"].as_str().unwrap_or("").len())], &c.state["theirs"].as_str().unwrap_or("")[..8.min(c.state["theirs"].as_str().unwrap_or("").len())]);
    }
    Ok(())
}

fn verify(dir: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let (n, bad) = peer.store.scan()?;
    println!("{{\"event\":\"verify\",\"objects\":{n},\"damaged\":{:?}}}", bad.iter().map(|b| b.short()).collect::<Vec<_>>());
    Ok(())
}

fn corrupt(dir: &Path) -> Result<()> {
    // Damage the newest version object: flip one byte, as a failing disk might.
    let peer = Peer::open(dir)?;
    let h = peer.head().context("no head")?;
    peer.store.damage_for_test(&h)?;
    println!("{{\"event\":\"corrupt\",\"object\":\"{}\"}}", h.short());
    Ok(())
}

/// Repair: drop damaged objects; the next pull from any peer fetches clean copies.
fn repair(dir: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let (_, bad) = peer.store.scan()?;
    for id in &bad {
        peer.store.remove(id)?;
    }
    println!("{{\"event\":\"repair\",\"removed_damaged\":{}}}", bad.len());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_default();
    let dir = arg(&args, "--dir").map(PathBuf::from);
    let relay_only = args.iter().any(|a| a == "--relay-only");
    let need_dir = || dir.clone().context("--dir is required");
    match cmd.as_str() {
        "setup" => setup(
            Path::new(&arg(&args, "--root").context("--root")?),
            &args_after(&args, "--photos"),
            arg(&args, "--video"),
            arg(&args, "--synthetic").map(|s| s.parse()).transpose()?.unwrap_or(0),
            arg(&args, "--extra-peers").map(|s| s.parse()).transpose()?.unwrap_or(0),
        )
        .await,
        "serve" => serve(&need_dir()?, relay_only).await,
        "pull" => pull(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), relay_only, args.iter().any(|a| a == "--closure")).await,
        "fetch-media" => {
            let h = arg(&args, "--hash").context("--hash")?;
            if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("--hash must be 64 hex characters, got {h:?}");
            }
            fetch_media(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), &h, relay_only).await
        }
        "edit" => edit(
            &need_dir()?,
            &arg(&args, "--key").context("--key")?,
            &arg(&args, "--field").context("--field")?,
            arg(&args, "--value").context("--value")?.parse()?,
            &arg(&args, "--message").unwrap_or_else(|| "Edit".into()),
        ),
        "status" => status(&need_dir()?),
        "reset" => {
            // Lab only: empty this peer (keeps its key and certificate), re-seed the genesis from `--like`.
            let d = need_dir()?;
            let like = PathBuf::from(arg(&args, "--like").context("--like <peer dir holding the project>")?);
            let src = RedbStore::open(like.join("kopano.redb"))?;
            let pid = Id::parse(std::fs::read_to_string(like.join("project"))?.trim())?;
            let genesis = src.get(&pid)?;
            drop(src);
            let _ = std::fs::remove_file(d.join("kopano.redb"));
            let _ = std::fs::remove_file(d.join("head"));
            std::fs::write(d.join("app.json"), b"{}")?;
            let s = RedbStore::open(d.join("kopano.redb"))?;
            s.put_verified(&pid, &genesis)?;
            s.flush()?;
            println!("{{\"event\":\"reset\"}}");
            Ok(())
        }
        "verify" => verify(&need_dir()?),
        "corrupt" => corrupt(&need_dir()?),
        "repair" => repair(&need_dir()?),
        "hash-of" => {
            let peer = Peer::open(&need_dir()?)?;
            let app = peer.app()?;
            let key = arg(&args, "--key").context("--key")?;
            let item = app.photos.get(&key).context("no such item")?;
            let a: &AssetRef = item.assets.first().context("no asset")?;
            println!("{}", a.hash);
            Ok(())
        }
        _ => bail!("unknown command {cmd:?}; see the file header"),
    }
}
