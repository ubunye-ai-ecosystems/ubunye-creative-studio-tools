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
//!   setup   --root R --photos <file>... --video <file> [--cert-days N]   create provider, alice, bob, project
//!   serve   --dir D [--relay-only]                     run until killed; writes D/addr.json
//!   pull    --dir D --from <addr.json> [--relay-only]  fetch + verify + integrate the peer's head
//!   fetch-media --dir D --from <addr.json> --hash H [--relay-only]   get one original
//!   edit    --dir D --key K --field F --value V        change a setting in the toy app and save
//!   status  --dir D
//!   corrupt --dir D                                    damage one stored object (recovery test)
//!   verify  --dir D                                    re-hash every object, list damaged ones

#[path = "../keystore.rs"]
mod keystore;
use anyhow::{anyhow, bail, Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use kopano_core::connector::Connector;
use kopano_core::identity::{now_ms, DeviceCert, DeviceKey, Provider};
use kopano_core::project::{self, AssetRef, Item, Member, Project, Role};
use kopano_core::repo::{Integrated, Repo};
use kopano_core::store::{Id, ObjectStore, RedbStore};
use kopano_core::tree;
use kopano_lab::ToyEditor;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
        let key_bytes = keystore::load(dir)?;
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
        if lan_only() {
            // E32/E35: no internet at all. No relay, no address lookup: only the addresses in the
            // teammate's addr file (a QR code in an app), over the local network.
            let mut b = Endpoint::builder(presets::Minimal).secret_key(sk).alpns(alpns).relay_mode(iroh::endpoint::RelayMode::Disabled);
            if let Some(t) = transport() {
                b = b.transport_config(t);
            }
            return Ok(b.bind().await?);
        }
        let mut b = Endpoint::builder(presets::N0).secret_key(sk).alpns(alpns);
        if let Some(t) = transport() {
            b = b.transport_config(t);
        }
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

/// E35: QUIC's default congestion control reads every lost packet as "the network is full" and
/// slows down; on lossy radio links that throttles for nothing. KOPANO_CC=bbr uses BBRv3, which
/// paces by measured bandwidth and delay instead. E35b: at 1% loss BBRv3 made a 50 MB download 34x
/// faster and a join 6x faster, at no cost on a clean link, so it is the default.
/// KOPANO_CC=default (iroh's own), cubic or reno for comparisons.
fn transport() -> Option<iroh::endpoint::QuicTransportConfig> {
    let cc: Arc<dyn iroh::endpoint::ControllerFactory + Send + Sync> = match std::env::var("KOPANO_CC").as_deref() {
        Ok("default") => return None,
        Ok("cubic") => Arc::new(noq_proto::congestion::CubicConfig::default()),
        Ok("reno") => Arc::new(noq_proto::congestion::NewRenoConfig::default()),
        _ => Arc::new(noq_proto::congestion::Bbr3Config::default()),
    };
    Some(iroh::endpoint::QuicTransportConfig::builder().congestion_controller_factory(cc).build())
}

fn lan_only() -> bool {
    std::env::var("KOPANO_LAN_ONLY").is_ok_and(|v| v == "1")
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
    // A frame that starts must finish at a living pace (security pass: a slow sender held memory
    // and a connection slot forever). 64 MiB at 100 kbit/s would take 90 minutes, so the deadline
    // grows with the size: 30 s plus 1 s per 16 KiB.
    let deadline = std::time::Duration::from_secs(30 + (n / 16_384) as u64);
    let mut buf = vec![0u8; n];
    tokio::time::timeout(deadline, r.read_exact(&mut buf)).await.map_err(|_| anyhow!("frame of {n} bytes too slow"))??;
    Ok(buf)
}

// ---- server side ------------------------------------------------------------------------------

#[derive(Clone)]
struct SyncHandler {
    peer: Arc<Peer>,
    /// E20: new heads announced to watchers (the daemon pushes; plain `serve` never sends)
    heads: tokio::sync::broadcast::Sender<Id>,
    /// E24: one pushed payload per (what the watcher had, new head), shared by every watcher
    pushed: Arc<std::sync::Mutex<HashMap<(Vec<Id>, Id), Arc<(u64, usize, Vec<u8>)>>>>,
}
impl std::fmt::Debug for SyncHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncHandler")
    }
}

/// Security pass (flood test E26): connections that have not yet shown a valid certificate wait in
/// a small room of WAITING slots; when it is full the OLDEST waiter is pushed out, not the newcomer,
/// so a member's handshake (one round trip) gets through unless a stranger opens WAITING new
/// connections within that round trip. Each waiter has HANDSHAKE_S to show a certificate.
/// Members who got in share MAX_MEMBERS slots.
const WAITING: usize = 16;
const HANDSHAKE_S: u64 = 5;
const MAX_MEMBERS: usize = 64;
static MEMBERS_IN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ROOM: std::sync::Mutex<std::collections::VecDeque<(u64, Connection)>> = std::sync::Mutex::new(std::collections::VecDeque::new());
static TICKETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn room_enter(conn: &Connection) -> u64 {
    let t = TICKETS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut room = ROOM.lock().unwrap_or_else(|e| e.into_inner());
    while room.len() >= WAITING {
        if let Some((_, old)) = room.pop_front() {
            old.close(2u32.into(), b"crowded");
        }
    }
    room.push_back((t, conn.clone()));
    t
}
fn room_leave(ticket: u64) {
    ROOM.lock().unwrap_or_else(|e| e.into_inner()).retain(|(t, _)| *t != ticket);
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let ticket = room_enter(&conn);
        let mut member = false;
        if let Err(e) = self.serve(conn, ticket, &mut member).await {
            eprintln!("[serve] connection ended: {e:#}");
        }
        room_leave(ticket);
        if member {
            MEMBERS_IN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

impl SyncHandler {
    async fn serve(&self, conn: Connection, ticket: u64, member: &mut bool) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let hello = async {
            let (send, mut recv) = conn.accept_bi().await?;
            let cert = read_frame(&mut recv).await?;
            anyhow::Ok((send, recv, cert))
        };
        let (mut send, mut recv, cert) = tokio::time::timeout(std::time::Duration::from_secs(HANDSHAKE_S), hello).await.map_err(|_| anyhow!("no certificate within {HANDSHAKE_S} s"))??;
        // 1. Mutual identity: they show their certificate first, we check it, then show ours.
        let their: DeviceCert = serde_json::from_slice(&cert)?;
        if let Err(e) = self.peer.check_peer(&their, &remote) {
            write_frame(&mut send, format!("refused: {e}").as_bytes()).await?;
            send.finish()?;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), send.stopped()).await;
            bail!("refused {}: {e}", their.body.name);
        }
        room_leave(ticket);
        if MEMBERS_IN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_MEMBERS {
            MEMBERS_IN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            conn.close(1u32.into(), b"busy");
            bail!("too many members connected");
        }
        *member = true;
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
                Some("watch") => {
                    // push: each new head goes out WITH the change (compact, relative to what this
                    // watcher already has), so the watcher applies it without asking: half a round trip.
                    let mut rx = self.heads.subscribe();
                    let mut known: Vec<Id> = req.get("have").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).filter(|h| self.peer.store.has(h)).collect();
                    let mut next = self.peer.head();
                    loop {
                        if let Some(h) = next {
                            if !known.contains(&h) {
                                let cached = self.pushed.lock().unwrap().get(&(known.clone(), h)).cloned();
                                let built = match cached {
                                    Some(b) => b,
                                    None => {
                                        let b = Arc::new(compact_payload(&self.peer, &known)?);
                                        let mut c = self.pushed.lock().unwrap();
                                        if c.len() > 256 {
                                            c.clear();
                                        }
                                        c.insert((known.clone(), h), b.clone());
                                        b
                                    }
                                };
                                let (records, raw, z) = (built.0, built.1, &built.2);
                                write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"head": h.hex(), "ms": now_ms(), "records": records, "raw": raw}))?).await?;
                                write_frame(&mut send, z).await?;
                                known = vec![h];
                            }
                        }
                        match rx.recv().await {
                            Ok(h) => next = Some(h),
                            Err(_) => break,
                        }
                    }
                    break;
                }
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
                Some("closure2") => {
                    let have: Vec<Id> = req.get("have").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|h| h.as_str().and_then(|s| Id::parse(s).ok())).filter(|h| self.peer.store.has(h)).collect();
                    let (records, raw, z) = compact_payload(&self.peer, &have)?;
                    write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"records": records, "raw": raw, "compressed": z.len()}))?).await?;
                    write_frame(&mut send, &z).await?;
                }
                _ => bail!("unknown request"),
            }
        }
        Ok(())
    }
}


/// E19/E20 compact reply: oldest version first; changed tree nodes as deltas from the node at the
/// same place in the parent tree; everything deflated. Returns (records, raw bytes, compressed).
fn compact_payload(peer: &Peer, have: &[Id]) -> Result<(u64, usize, Vec<u8>)> {
    let known: BTreeSet<Id> = project::history(&peer.store, have)?.into_iter().map(|(id, _)| id).collect();
    let mut versions: Vec<(Id, project::Version)> = project::history(&peer.store, &peer.head().into_iter().collect::<Vec<_>>())?.into_iter().filter(|(id, _)| !known.contains(id)).collect();
    versions.reverse();
    let mut sent: HashSet<Id> = HashSet::new();
    let mut payload: Vec<u8> = Vec::new();
    let mut records = 0u64;
    let put = |kind: u8, data: &[u8], payload: &mut Vec<u8>| {
        payload.push(kind);
        payload.extend((data.len() as u32).to_le_bytes());
        payload.extend(data);
    };
    for (vid, v) in &versions {
        let parent_tree = match v.parents()?.first() {
            Some(p) => project::load_version(&peer.store, p)?.tree()?,
            None => None,
        };
        let mut pairs = vec![];
        tree::delta_bases(&peer.store, v.tree()?, parent_tree, &mut pairs)?;
        let mut nodes: HashSet<Id> = HashSet::new();
        for (nid, base) in &pairs {
            nodes.insert(*nid);
            if !sent.insert(*nid) {
                continue;
            }
            let nb = peer.store.get(nid)?;
            match base.and_then(|b| peer.store.get(&b).ok().and_then(|ob| tree::node_delta(&ob, &nb).map(|d| (b, d)))) {
                Some((b, d)) => {
                    let mut rec = b.0.to_vec();
                    rec.extend(d);
                    put(1, &rec, &mut payload);
                }
                None => put(0, &nb, &mut payload),
            }
            records += 1;
        }
        let mut miss = vec![];
        tree::missing(&peer.store, v.tree()?, parent_tree, &mut miss)?;
        for id in miss {
            if nodes.contains(&id) || !sent.insert(id) {
                continue;
            }
            put(0, &peer.store.get(&id)?, &mut payload);
            records += 1;
        }
        put(0, &peer.store.get(vid)?, &mut payload);
        sent.insert(*vid);
        records += 1;
    }
    let z = miniz_oxide::deflate::compress_to_vec(&payload, 6);
    Ok((records, payload.len(), z))
}

/// E37c: a refused device used to read "that teammate does not have this file", which sent people
/// looking for the file when the real cause was an expired device certificate.
fn media_refusal(e: impl std::fmt::Display) -> anyhow::Error {
    let e = e.to_string();
    if e.contains("permission") {
        anyhow!("that teammate refused this device: its device certificate is expired or not a member's; sign in again to renew it ({e})")
    } else {
        anyhow!("that teammate does not have this file (yet); try another teammate or the owner ({e})")
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
    let router = Router::builder(ep).accept(SYNC_ALPN, SyncHandler { peer: peer.clone(), heads: tokio::sync::broadcast::channel(64).0, pushed: Default::default() }).accept(iroh_blobs::ALPN, blobs_proto).spawn();
    let t = Instant::now();
    // E35: "online" means a relay answered. With no internet that never happens, and the device
    // must still work on the local network, so wait at most 10 s (not at all in LAN-only mode).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(if lan_only() { 0 } else { 10 }), router.endpoint().online()).await;
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
    if lan_only() {
        addr = EndpointAddr::from_parts(addr.id, addr.ip_addrs().cloned().map(iroh::TransportAddr::Ip));
    }
    let ep = peer.endpoint(relay_only, vec![]).await?;
    let t = Instant::now();
    let conn = ep.connect(addr, alpn).await?;
    let ms = t.elapsed().as_millis();
    let path = conn.paths().iter().find(|p| p.is_selected()).map(|p| format!("{:?}", p.remote_addr())).unwrap_or_else(|| "unknown".into());
    Ok((ep, conn, ms, path))
}

/// Security pass: a stranger (fresh key, no certificate) opens `n` sync connections and sends
/// nothing, to see whether members can still get in. Lab only.
async fn flood(from: &Path, n: usize, hold_s: u64) -> Result<()> {
    let addr: EndpointAddr = serde_json::from_slice(&std::fs::read(from)?)?;
    let mut held = vec![];
    let (mut opened, mut refused) = (0, 0);
    // keep opening new silent connections until `hold_s` is over (at most n held at once)
    let end = Instant::now() + std::time::Duration::from_secs(hold_s);
    while Instant::now() < end {
        if held.len() >= n {
            held.remove(0);
        }
        let sk = SecretKey::from_bytes(&DeviceKey::generate().to_bytes());
        let ep = Endpoint::builder(presets::N0).secret_key(sk).bind().await?;
        match ep.connect(addr.clone(), SYNC_ALPN).await {
            Ok(c) => match c.open_bi().await {
                Ok((mut s, r)) => {
                    let _ = s.write_all(&[0, 0]).await; // half a length prefix, then silence
                    opened += 1;
                    held.push((ep, c, s, r));
                }
                Err(_) => refused += 1,
            },
            Err(_) => refused += 1,
        }
    }
    let alive = held.iter().filter(|(_, c, _, _)| c.close_reason().is_none()).count();
    println!("{}", serde_json::json!({"event": "flood", "seconds": hold_s, "opened": opened, "refused": refused, "alive_at_end": alive}));
    Ok(())
}

async fn pull(dir: &Path, from: &Path, relay_only: bool, closure: bool, compact: bool) -> Result<()> {
    let closure = closure || compact;
    let peer = Peer::open(dir)?;
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, SYNC_ALPN).await?;
    let t_all = Instant::now();
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, &serde_json::to_vec(&peer.cert)?).await?;
    if closure {
        // Pipelined: certificate, heads and closure requests leave in one flight (one round trip).
        write_frame(&mut send, br#"{"op":"heads"}"#).await?;
        let have: Vec<String> = peer.head().into_iter().map(|h| h.hex()).collect();
        let op = if compact { "closure2" } else { "closure" };
        write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"op": op, "have": have}))?).await?;
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
        if compact {
            let raw = hdr.get("raw").and_then(|n| n.as_u64()).unwrap_or(0);
            let z = read_frame(&mut recv).await?;
            bytes += z.len() as u64;
            let d = kopano_core::wire::decode_compact(&peer.store, &z, raw as u64)?;
            objects += d.objects;
            new_versions.extend(d.versions);
            // records come oldest first; the check below walks the list backwards
            new_versions.reverse();
        }
        let n = if compact { 0 } else { hdr.get("n").and_then(|n| n.as_u64()).unwrap_or(0).min(10_000_000) };
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
    let transfer_ms = t_all.elapsed().as_millis();
    // E20: the cost of further syncs on the connection already open (a long-running service).
    let repeat: usize = std::env::var("KOPANO_REPEAT").ok().and_then(|r| r.parse().ok()).unwrap_or(0);
    if repeat > 0 && compact {
        let have: Vec<String> = their_heads.iter().map(|h| h.hex()).collect();
        let mut times = vec![];
        for _ in 0..repeat {
            let t = Instant::now();
            write_frame(&mut send, &serde_json::to_vec(&serde_json::json!({"op": "closure2", "have": have}))?).await?;
            let _hdr = read_frame(&mut recv).await?;
            let _z = read_frame(&mut recv).await?;
            times.push(t.elapsed().as_millis() as u64);
        }
        println!("{{\"event\":\"open-connection syncs\",\"ms\":{times:?}}}");
    }
    send.finish()?;
    conn.close(0u32.into(), b"done");
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

async fn fetch_media(dir: &Path, from: &Path, hash: &str, relay_only: bool, out: Option<PathBuf>) -> Result<()> {
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let before = blobs.remote().local(h).await?;
    let had = before.local_bytes();
    if before.is_complete() {
        if let Some(o) = &out {
            save_blob(&blobs, h, o).await?;
        }
        println!("{{\"event\":\"media\",\"hash\":\"{}\",\"already_complete\":true,\"bytes_received\":0}}", &hash[..8]);
        blobs.shutdown().await?;
        return Ok(());
    }
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, iroh_blobs::ALPN).await?;
    // E28: learn the size first (the last chunk proves it), and make sure the rest fits on this
    // disk before starting, instead of failing part way with an obscure error.
    {
        // in memory only: the store reserves the whole file on its first write, so nothing may
        // be written before the check
        let (size, _) = iroh_blobs::get::request::get_verified_size(&conn, &h).await.map_err(media_refusal)?;
        let need = size.saturating_sub(had) + (16 << 20);
        if let Err(e) = reserve(&peer.dir.join("media"), need) {
            conn.close(0u32.into(), b"no space");
            ep.close().await;
            bail!("not enough disk space: this original needs {} MB more ({e}). Free some space and run it again; nothing already received is lost.", need / 1_000_000);
        }
    }
    let t = Instant::now();
    let stats = blobs.remote().fetch(conn.clone(), h).complete().await?;
    let ms = t.elapsed().as_millis();
    let after = blobs.remote().local(h).await?;
    conn.close(0u32.into(), b"done");
    ep.close().await;
    if let (Some(o), true) = (&out, after.is_complete()) {
        save_blob(&blobs, h, o).await?;
    }
    println!(
        "{{\"event\":\"media\",\"hash\":\"{}\",\"path\":{:?},\"connect_ms\":{connect_ms},\"had_before\":{had},\"bytes_received\":{},\"total_read\":{},\"ms\":{ms},\"mbit_s\":{:.0},\"complete\":{}}}",
        &hash[..8],
        path,
        stats.payload_bytes_read,
        stats.total_bytes_read(),
        stats.mbits(),
        after.is_complete()
    );
    // E38: close the store, or what was just received may not be kept after the program exits
    blobs.shutdown().await?;
    Ok(())
}

/// E30: fetch only some byte ranges of a remote original (what a timeline needs), each 1 KiB chunk
/// verified against the original's BLAKE3 root by iroh-blobs (bao), and write them into a sparse
/// file the size of the original. What was already fetched is not fetched again.
async fn fetch_ranges(dir: &Path, from: &Path, hash: &str, ranges: &[(u64, u64)], out: &Path, relay_only: bool) -> Result<()> {
    use iroh_blobs::protocol::{ChunkRanges, ChunkRangesExt, GetRequest};
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let mut want = ChunkRanges::last_chunk(); // the size, proven by the last chunk
    for (a, b) in ranges {
        want |= ChunkRanges::bytes(*a..*b);
    }
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, iroh_blobs::ALPN).await?;
    let t = Instant::now();
    let stats = blobs.remote().execute_get(conn.clone(), GetRequest::blob_ranges(h, want)).complete().await?;
    let ms = t.elapsed().as_millis();
    conn.close(0u32.into(), b"done");
    ep.close().await;
    // write what we hold for the asked ranges into a sparse copy
    let out = std::path::absolute(out)?;
    let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&out)?;
    let mut size = 0u64;
    let mut written = 0u64;
    for (a, b) in ranges {
        let mut s = blobs.blobs().export_ranges(h, *a..*b).stream();
        use futures_lite::StreamExt;
        while let Some(item) = s.next().await {
            match item {
                iroh_blobs::api::proto::ExportRangesItem::Size(n) => {
                    size = n;
                    if f.metadata()?.len() != n {
                        f.set_len(n)?;
                    }
                }
                iroh_blobs::api::proto::ExportRangesItem::Data(leaf) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::FileExt;
                        f.seek_write(&leaf.data, leaf.offset)?;
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::FileExt;
                        f.write_all_at(&leaf.data, leaf.offset)?;
                    }
                    written += leaf.data.len() as u64;
                }
                iroh_blobs::api::proto::ExportRangesItem::Error(e) => bail!("export: {e}"),
            }
        }
    }
    println!(
        "{{\"event\":\"ranges\",\"hash\":\"{}\",\"path\":{:?},\"connect_ms\":{connect_ms},\"file_size\":{size},\"asked_bytes\":{},\"bytes_received\":{},\"total_read\":{},\"written\":{written},\"ms\":{ms}}}",
        &hash[..8],
        path,
        ranges.iter().map(|(a, b)| b - a).sum::<u64>(),
        stats.payload_bytes_read,
        stats.total_bytes_read()
    );
    blobs.shutdown().await?;
    Ok(())
}

/// Check that `bytes` fit on the disk holding `dir` by briefly reserving them (Windows allocates
/// on set_len; elsewhere the OS may create a sparse file, so this only catches some cases).
fn reserve(dir: &Path, bytes: u64) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join(".space-check");
    let r = std::fs::File::create(&p).and_then(|f| f.set_len(bytes));
    let _ = std::fs::remove_file(&p);
    r.map_err(|e| anyhow!("{e}"))
}

/// Write a verified blob out as an ordinary file.
async fn save_blob(blobs: &iroh_blobs::store::fs::FsStore, h: iroh_blobs::Hash, out: &Path) -> Result<()> {
    let out = std::path::absolute(out)?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    blobs.blobs().export(h, &out).await?;
    eprintln!("[media] saved to {}", out.display());
    Ok(())
}

/// Make the "final image" of one photo from this device's own synced settings (lab check that
/// devices agree): the camera's embedded JPEG, with exposure, contrast and white balance applied.
async fn render(dir: &Path, key: &str, out: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let app = peer.app()?;
    let item = app.photos.get(key).with_context(|| format!("no photo {key}"))?;
    let d = &item.state["develop"];
    let ev = d["exposure"].as_f64().unwrap_or(0.0) as f32;
    let contrast = d["contrast"].as_f64().unwrap_or(0.0) as f32;
    let temp = d["temperature"].as_f64().unwrap_or(5500.0) as f32;
    let sat = d["saturation"].as_f64().unwrap_or(100.0) as f32 / 100.0;
    let hash = &item.assets.first().context("no original")?.hash;
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    if !blobs.remote().local(h).await?.is_complete() {
        bail!("the original of {key} is not on this device yet; fetch it first");
    }
    let tmp = std::path::absolute(dir.join("render-original.tmp"))?;
    blobs.blobs().export(h, &tmp).await?;
    blobs.shutdown().await?;
    let bytes = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    // the largest JPEG the camera embedded in the raw file
    let mut best: Option<image::DynamicImage> = None;
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF {
            if let Ok(img) = image::load_from_memory_with_format(&bytes[i..], image::ImageFormat::Jpeg) {
                if best.as_ref().map_or(true, |b| img.width() * img.height() > b.width() * b.height()) {
                    best = Some(img);
                }
            }
        }
        i += 1;
        if best.as_ref().is_some_and(|b| b.width() >= 4000) {
            break;
        }
    }
    let mut img = best.context("no embedded JPEG in the original")?.to_rgb8();
    // the camera's orientation tag (TIFF IFD0, tag 0x0112) so portrait photos stand upright
    let orient = (|| -> Option<u16> {
        let le = bytes.get(0..2)? == b"II";
        let u16_at = |o: usize| bytes.get(o..o + 2).map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
        let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) });
        let ifd = u32_at(4)? as usize;
        let n = u16_at(ifd)? as usize;
        (0..n.min(200)).find_map(|i| {
            let e = ifd + 2 + i * 12;
            (u16_at(e)? == 0x0112).then(|| u16_at(e + 8)).flatten()
        })
    })()
    .unwrap_or(1);
    img = match orient {
        3 => image::imageops::rotate180(&img),
        6 => image::imageops::rotate90(&img),
        8 => image::imageops::rotate270(&img),
        _ => img,
    };
    let (w, hgt) = (img.width() as f32, img.height() as f32);
    let k = 1600.0 / w.max(hgt);
    let mut img = image::imageops::resize(&img, (w * k).round() as u32, (hgt * k).round() as u32, image::imageops::FilterType::Triangle);
    let gain = 2f32.powf(ev);
    let shift = (temp - 5500.0) / 8000.0;
    let (rk, bk) = ((1.0 + shift).max(0.0), (1.0 - shift).max(0.0));
    let ck = 1.0 + contrast / 100.0;
    for p in img.pixels_mut() {
        let mut v = [0f32; 3];
        for (c, m) in [(0usize, rk), (1, 1.0), (2, bk)] {
            let lin = (p[c] as f32 / 255.0).powf(2.2) * gain * m;
            let s = lin.clamp(0.0, 1.0).powf(1.0 / 2.2);
            v[c] = ((s - 0.5) * ck + 0.5).clamp(0.0, 1.0);
        }
        let luma = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        for c in 0..3 {
            p[c] = ((luma + (v[c] - luma) * sat).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    let out = std::path::absolute(out)?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut f = std::fs::File::create(&out)?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 88).encode_image(&img)?;
    drop(f);
    let fp = blake3::hash(&std::fs::read(&out)?).to_hex().to_string();
    println!("{{\"event\":\"render\",\"key\":\"{key}\",\"exposure\":{ev},\"contrast\":{contrast},\"temperature\":{temp},\"saturation\":{},\"width\":{},\"height\":{},\"fingerprint\":\"{}\"}}", sat * 100.0, img.width(), img.height(), &fp[..16]);
    Ok(())
}

/// Add a new photo to the project: the original goes into this device's media store, the photo
/// (default settings, referencing the original by hash) into the app, saved as a new version.
async fn import(dir: &Path, file: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let tag = blobs.blobs().add_path(std::path::absolute(file)?).await?;
    blobs.shutdown().await?;
    let key = file.file_name().and_then(|s| s.to_str()).context("file name")?.to_string();
    let size = std::fs::metadata(file)?.len();
    let mut app = peer.app()?;
    let fresh = ToyEditor::new(&[(key.clone(), tag.hash.to_hex().to_string(), size)]);
    app.photos.extend(fresh.photos);
    let mut repo = peer.repo()?;
    let v = repo.save(&mut app, &format!("Imported {key}"))?;
    peer.save_app(&app)?;
    peer.set_head(repo.head)?;
    println!("{{\"event\":\"import\",\"key\":\"{key}\",\"hash\":\"{}\",\"bytes\":{size},\"version\":\"{}\"}}", tag.hash.to_hex(), v.map(|v| v.short()).unwrap_or_default());
    Ok(())
}

/// Offer any file to the team as media (lab): prints its BLAKE3 hash.
// ---- E32: bundles, for when there is no network at all (USB stick, SD card, a phone carried) --
//
// File: "KOPANO-BUNDLE-1\n", one JSON header line, the compact payload (as on the wire), then
// for each original: 8-byte length, 32-byte BLAKE3 hash, the bytes. On import every object is
// stored under its own hash, every version is verified like a network pull, and every original is
// re-hashed; a damaged or forged bundle changes nothing.

const BUNDLE_MAGIC: &[u8] = b"KOPANO-BUNDLE-1\n";

async fn export_bundle(dir: &Path, have: Option<String>, media: &[String], out: &Path) -> Result<()> {
    use std::io::Write;
    let peer = Peer::open(dir)?;
    let have: Vec<Id> = have.iter().filter_map(|h| Id::parse(h.trim()).ok()).filter(|h| peer.store.has(h)).collect();
    let (records, raw, z) = compact_payload(&peer, &have)?;
    let head = peer.head().context("nothing to export")?;
    let hdr = serde_json::json!({"project": peer.project.id.hex(), "head": head.hex(), "from": peer.cert.body.name, "records": records, "raw": raw, "compressed": z.len(), "media": media.len()});
    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    f.write_all(BUNDLE_MAGIC)?;
    f.write_all(format!("{hdr}\n").as_bytes())?;
    f.write_all(&z)?;
    let blobs = peer.blobs().await?;
    let mut media_bytes = 0u64;
    for m in media {
        let h: iroh_blobs::Hash = m.parse()?;
        let tmp = std::path::absolute(dir.join(format!(".bundle-{}", &m[..8])))?;
        blobs.blobs().export(h, &tmp).await?;
        let len = std::fs::metadata(&tmp)?.len();
        f.write_all(&len.to_le_bytes())?;
        f.write_all(h.as_bytes())?;
        std::io::copy(&mut std::fs::File::open(&tmp)?, &mut f)?;
        std::fs::remove_file(&tmp)?;
        media_bytes += len;
    }
    f.flush()?;
    f.into_inner()?.sync_all()?;
    blobs.shutdown().await?;
    println!("{}", serde_json::json!({"event": "bundle-out", "head": head.short(), "records": records, "compressed": z.len(), "media": media.len(), "media_bytes": media_bytes, "file_bytes": std::fs::metadata(out)?.len()}));
    Ok(())
}

async fn import_bundle(dir: &Path, file: &Path) -> Result<()> {
    use std::io::{BufRead, Read};
    let peer = Peer::open(dir)?;
    let t = Instant::now();
    // a sealed bundle is opened with this device's key first (only member devices can)
    let mut probe = [0u8; 16];
    let sealed = std::fs::File::open(file)?.read(&mut probe)? == kopano_core::seal::MAGIC.len() && probe.as_slice() == kopano_core::seal::MAGIC;
    let opened = dir.join(".bundle-opened");
    let file: &Path = if sealed {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&opened)?);
        kopano_core::seal::open(&mut std::io::BufReader::new(std::fs::File::open(file)?), &mut w, peer.key.signing())?;
        w.into_inner()?.sync_all()?;
        &opened
    } else {
        file
    };
    let mut f = std::io::BufReader::new(std::fs::File::open(file)?);
    let mut magic = vec![0u8; BUNDLE_MAGIC.len()];
    f.read_exact(&mut magic)?;
    if magic != BUNDLE_MAGIC {
        bail!("not a Kopano bundle");
    }
    let mut line = String::new();
    f.by_ref().take(64 * 1024).read_line(&mut line)?;
    let hdr: serde_json::Value = serde_json::from_str(&line).context("bad bundle header")?;
    if hdr["project"].as_str() != Some(&peer.project.id.hex()) {
        bail!("this bundle is for another project");
    }
    let zlen = hdr["compressed"].as_u64().context("header")?;
    if zlen > MAX_FRAME as u64 {
        bail!("bundle payload too large");
    }
    let mut z = vec![0u8; zlen as usize];
    f.read_exact(&mut z)?;
    const DAMAGED: &str = "this bundle is damaged or was changed after it was made; nothing was imported. Copy it again from the sender";
    let d = kopano_core::wire::decode_compact(&peer.store, &z, hdr["raw"].as_u64().unwrap_or(0)).map_err(|_| anyhow!(DAMAGED))?;
    for v in &d.versions {
        project::verify_version(&peer.store, &peer.project, v).map_err(|_| anyhow!(DAMAGED))?;
    }
    let head = Id::parse(hdr["head"].as_str().context("header")?)?;
    let mut app = peer.app()?;
    let mut repo = peer.repo()?;
    let result = match repo.integrate(&mut app, &head)? {
        Integrated::AlreadyHad => "already up to date".to_string(),
        Integrated::FastForward { applied } => format!("fast-forward, {applied} item(s) applied"),
        Integrated::Merged { applied, conflicts, .. } => format!("merged, {applied} item(s) applied, {} clash(es)", conflicts.len()),
    };
    peer.save_app(&app)?;
    peer.set_head(repo.head)?;
    peer.store.flush()?;
    // originals: each re-hashed before it is kept
    let blobs = peer.blobs().await?;
    let (mut ok, mut bad) = (0u32, 0u32);
    for _ in 0..hdr["media"].as_u64().unwrap_or(0) {
        let mut len = [0u8; 8];
        let mut want = [0u8; 32];
        f.read_exact(&mut len)?;
        f.read_exact(&mut want)?;
        let len = u64::from_le_bytes(len);
        let tmp = std::path::absolute(dir.join(".bundle-in"))?;
        let copied = std::io::copy(&mut f.by_ref().take(len), &mut std::fs::File::create(&tmp)?)?;
        if copied != len {
            let _ = std::fs::remove_file(&tmp);
            bail!("bundle is cut short");
        }
        let tag = blobs.blobs().add_path(&tmp).await?;
        std::fs::remove_file(&tmp)?;
        if tag.hash.as_bytes() == &want {
            ok += 1;
        } else {
            bad += 1; // stored under its true hash, which nothing refers to; never used as the original
        }
    }
    blobs.shutdown().await?;
    drop(f);
    let _ = std::fs::remove_file(&opened);
    println!("{}", serde_json::json!({"event": "bundle-in", "sealed": sealed, "objects": d.objects, "versions": d.versions.len(), "result": result, "media_ok": ok, "media_rejected": bad, "ms": t.elapsed().as_millis()}));
    Ok(())
}

// ---- E31: one big original from several teammates at once ------------------------------------
//
// The file is cut into stripes; every teammate who holds it serves stripes from one shared queue
// (work stealing), so a fast teammate does more and a slow one less. A stripe whose teammate goes
// away goes back to the queue. Every 1 KiB chunk is verified against the original's hash, so a
// teammate cannot slip in wrong bytes, and stripes already held are never fetched again.

async fn fetch_swarm(dir: &Path, from: &str, hash: &str, stripe_mb: u64, relay_only: bool) -> Result<()> {
    use iroh_blobs::protocol::{ChunkRanges, ChunkRangesExt, GetRequest};
    let peer = Arc::new(Peer::open(dir)?);
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let sources: Vec<PathBuf> = from.split(',').map(PathBuf::from).collect();
    let t = Instant::now();
    // ONE endpoint for all teammates: the same device key on two endpoints at once makes the
    // relay drop one of them (found the hard way, E31)
    let ep = peer.endpoint(relay_only, vec![]).await?;
    let mut conns = vec![];
    for src in &sources {
        let mut addr: EndpointAddr = serde_json::from_slice(&std::fs::read(src)?)?;
        if relay_only {
            addr = EndpointAddr::from_parts(addr.id, addr.relay_urls().cloned().map(iroh::TransportAddr::Relay));
        }
        match ep.connect(addr, iroh_blobs::ALPN).await {
            Ok(c) => {
                let path = c.paths().iter().find(|p| p.is_selected()).map(|p| format!("{:?}", p.remote_addr())).unwrap_or_default();
                conns.push(((), c, 0u128, path));
            }
            Err(e) => eprintln!("[swarm] {} unreachable: {e:#}", src.display()),
        }
    }
    let (_, first, _, _) = conns.first().context("no teammate reachable")?;
    let (size, _) = iroh_blobs::get::request::get_verified_size(first, &h).await.map_err(media_refusal)?;
    let stripe = stripe_mb << 20;
    let total = size.div_ceil(stripe);
    let queue: Arc<std::sync::Mutex<Vec<u64>>> = Arc::new(std::sync::Mutex::new((0..total).rev().collect()));
    // endgame: when the queue is empty, an idle teammate also fetches a stripe someone slower is
    // still on; whichever copy lands first counts (both are verified, the store keeps one)
    let busy: Arc<std::sync::Mutex<BTreeSet<u64>>> = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let done: Arc<std::sync::Mutex<BTreeSet<u64>>> = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let got: Arc<std::sync::Mutex<Vec<(u64, u32)>>> = Arc::new(std::sync::Mutex::new(vec![(0, 0); conns.len()]));
    let mut tasks = vec![];
    for (i, (_ep, conn, _, _)) in conns.iter().enumerate() {
        let (queue, busy, done, got, blobs, conn) = (queue.clone(), busy.clone(), done.clone(), got.clone(), blobs.clone(), conn.clone());
        tasks.push(tokio::spawn(async move {
            loop {
                let next = queue.lock().unwrap().pop().or_else(|| {
                    let d = done.lock().unwrap();
                    busy.lock().unwrap().iter().copied().find(|k| !d.contains(k))
                });
                let Some(k) = next else { break };
                busy.lock().unwrap().insert(k);
                let range = ChunkRanges::bytes(k * stripe..((k + 1) * stripe).min(size));
                match blobs.remote().execute_get(conn.clone(), GetRequest::blob_ranges(h, range)).complete().await {
                    Ok(st) => {
                        let mut g = got.lock().unwrap();
                        g[i].0 += st.payload_bytes_read;
                        g[i].1 += 1;
                        done.lock().unwrap().insert(k);
                    }
                    Err(e) => {
                        busy.lock().unwrap().remove(&k);
                        if !done.lock().unwrap().contains(&k) {
                            queue.lock().unwrap().push(k); // someone else takes it
                        }
                        eprintln!("[swarm] teammate {i} stopped: {e:#}");
                        break;
                    }
                }
            }
        }));
    }
    // finished when every stripe landed once; stop whoever is still on a duplicate
    while done.lock().unwrap().len() < total as usize && tasks.iter().any(|t| !t.is_finished()) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    for t in &tasks {
        t.abort();
    }
    let per: Vec<serde_json::Value> = got.lock().unwrap().iter().enumerate().map(|(i, (b, n))| serde_json::json!({"teammate": i, "path": conns[i].3, "stripes": n, "mb": b / 1_000_000})).collect();
    let complete = blobs.remote().local(h).await?.is_complete();
    let ms = t.elapsed().as_millis();
    println!("{}", serde_json::json!({"event": "swarm", "size_mb": size / 1_000_000, "teammates": per.len(), "per_teammate": per, "complete": complete, "ms": ms, "mbit_s": (size as f64 * 8.0 / 1e6) / (ms as f64 / 1000.0)}));
    for (_, conn, _, _) in conns {
        conn.close(0u32.into(), b"done");
    }
    ep.close().await;
    blobs.shutdown().await?;
    Ok(())
}

// ---- E34: many small files as one collection: one request for all, not one per file -----------

async fn add_folder(dir: &Path, folder: &Path) -> Result<()> {
    use iroh_blobs::format::collection::Collection;
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let t = Instant::now();
    let mut items = vec![];
    let mut bytes = 0u64;
    let mut files: Vec<_> = std::fs::read_dir(folder)?.filter_map(|e| e.ok()).filter(|e| e.path().is_file()).collect();
    files.sort_by_key(|e| e.file_name());
    for e in files {
        let tag = blobs.blobs().add_path(std::path::absolute(e.path())?).await?;
        bytes += e.metadata()?.len();
        items.push((e.file_name().to_string_lossy().to_string(), tag.hash));
    }
    let n = items.len();
    let root = Collection::from_iter(items).store(&blobs).await?;
    println!("{}", serde_json::json!({"event": "add-folder", "hash": root.hash().to_hex(), "files": n, "bytes": bytes, "ms": t.elapsed().as_millis()}));
    blobs.shutdown().await?;
    Ok(())
}

async fn fetch_folder(dir: &Path, from: &Path, hash: &str, one_by_one: bool, relay_only: bool) -> Result<()> {
    use iroh_blobs::format::collection::Collection;
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let h: iroh_blobs::Hash = hash.parse()?;
    let (ep, conn, connect_ms, path) = connect(&peer, from, relay_only, iroh_blobs::ALPN).await?;
    let t = Instant::now();
    let (mut bytes, mut requests) = (0u64, 0u64);
    if one_by_one {
        // the naive way: the list, then one request per file
        bytes += blobs.remote().fetch(conn.clone(), iroh_blobs::HashAndFormat::raw(h)).complete().await?.payload_bytes_read;
        requests += 1;
        let seq: Vec<iroh_blobs::Hash> = iroh_blobs::hashseq::HashSeq::try_from(blobs.blobs().get_bytes(h).await?)?.into_iter().collect();
        for c in seq {
            bytes += blobs.remote().fetch(conn.clone(), iroh_blobs::HashAndFormat::raw(c)).complete().await?.payload_bytes_read;
            requests += 1;
        }
    } else {
        bytes += blobs.remote().fetch(conn.clone(), iroh_blobs::HashAndFormat::hash_seq(h)).complete().await?.payload_bytes_read;
        requests += 1;
    }
    let ms = t.elapsed().as_millis();
    conn.close(0u32.into(), b"done");
    ep.close().await;
    let files = Collection::load(h, &*blobs).await.map(|c| c.iter().count()).unwrap_or(0);
    println!("{}", serde_json::json!({"event": "fetch-folder", "one_by_one": one_by_one, "path": path, "connect_ms": connect_ms, "files": files, "requests": requests, "bytes": bytes, "ms": ms}));
    blobs.shutdown().await?;
    Ok(())
}

async fn add_media(dir: &Path, file: &Path) -> Result<()> {
    let peer = Peer::open(dir)?;
    let blobs = peer.blobs().await?;
    let tag = blobs.blobs().add_path(std::path::absolute(file)?).await?;
    println!("{{\"event\":\"add-media\",\"hash\":\"{}\",\"bytes\":{}}}", tag.hash.to_hex(), std::fs::metadata(file)?.len());
    blobs.shutdown().await?;
    Ok(())
}

// ---- setup and local commands -----------------------------------------------------------------

async fn setup(root: &Path, photos: &[String], video: Option<String>, synthetic: usize, extra_peers: usize, cert_days: u64) -> Result<()> {
    std::fs::create_dir_all(root)?;
    // E37c: one-day certificates and a provider key that is not kept left the real-device test team
    // locked out a day later; a test that runs for days needs `--cert-days`.
    let ttl = cert_days * 86_400_000;
    let provider = Provider::generate();
    std::fs::write(root.join("provider.pub"), provider.public_hex())?;
    let (ka, kb, kc) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
    let ca = provider.issue("acct-alice", "Alice", &ka, "studio-desktop", ttl)?;
    let cb = provider.issue("acct-bob", "Bob", &kb, "laptop", ttl)?;
    // Carol has a valid account and device, but is not a member of the project.
    let cc = provider.issue("acct-carol", "Carol", &kc, "laptop", ttl)?;
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
        let c = provider.issue(&format!("acct-m{i}"), &format!("Member{i}"), &k, "laptop", ttl)?;
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
    match item.state.pointer_mut(&ptr) {
        Some(v) => *v = serde_json::json!(value),
        None => {
            // a setting this photo does not have yet (e.g. saturation): add it to its group
            let (parent, name) = ptr.rsplit_once('/').with_context(|| format!("no setting {field}"))?;
            let obj = if parent.is_empty() { Some(&mut item.state) } else { item.state.pointer_mut(parent) };
            obj.and_then(|o| o.as_object_mut()).with_context(|| format!("no setting {field}"))?.insert(name.to_string(), serde_json::json!(value));
        }
    }
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

// ---- E20 daemon: one process holds the project, serves, takes app edits, pushes and follows ----

async fn open_sync(ep: Endpoint, addr: EndpointAddr, peer: Arc<Peer>) -> Result<(Connection, iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
    let conn = ep.connect(addr, SYNC_ALPN).await?;
    let (mut s, mut r) = conn.open_bi().await?;
    write_frame(&mut s, &serde_json::to_vec(&peer.cert)?).await?;
    let their: DeviceCert = serde_json::from_slice(&read_frame(&mut r).await?)?;
    peer.check_peer(&their, conn.remote_id().as_bytes())?;
    Ok((conn, s, r))
}

async fn follow(peer: Arc<Peer>, ep: Endpoint, from: PathBuf) -> Result<()> {
    let addr: EndpointAddr = serde_json::from_slice(&std::fs::read(&from)?)?;
    let (_wc, mut ws, mut wr) = open_sync(ep.clone(), addr.clone(), peer.clone()).await?;
    let have: Vec<String> = peer.head().into_iter().map(|h| h.hex()).collect();
    write_frame(&mut ws, &serde_json::to_vec(&serde_json::json!({"op": "watch", "have": have}))?).await?;
    eprintln!("[daemon] following, connection open");
    loop {
        let note: serde_json::Value = serde_json::from_slice(&read_frame(&mut wr).await?)?;
        let z = read_frame(&mut wr).await?;
        let t = Instant::now();
        let Some(h) = note.get("head").and_then(|h| h.as_str()).and_then(|h| Id::parse(h).ok()) else { continue };
        let raw = note.get("raw").and_then(|n| n.as_u64()).unwrap_or(0);
        let versions = kopano_core::wire::decode_compact(&peer.store, &z, raw)?.versions;
        for v in &versions {
            project::verify_version(&peer.store, &peer.project, v)?;
        }
        let mut app = peer.app()?;
        let mut repo = peer.repo()?;
        repo.integrate(&mut app, &h)?;
        peer.save_app(&app)?;
        peer.set_head(repo.head)?;
        peer.store.flush()?;
        println!(
            "{{\"event\":\"synced\",\"head\":\"{}\",\"bytes\":{},\"apply_ms\":{},\"edit_ms\":{},\"now_ms\":{}}}",
            h.short(),
            z.len(),
            t.elapsed().as_millis(),
            note.get("ms").and_then(|m| m.as_u64()).unwrap_or(0),
            now_ms()
        );
    }
}

async fn daemon(dir: &Path, port: u16, follow_from: Option<PathBuf>, relay_only: bool) -> Result<()> {
    let dir = &std::path::absolute(dir)?;
    let peer = Arc::new(Peer::open(dir)?);
    let blobs = peer.blobs().await?;
    let heads = tokio::sync::broadcast::channel::<Id>(64).0;
    let ep = peer.endpoint(relay_only, vec![SYNC_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()]).await?;
    let blobs_proto = iroh_blobs::BlobsProtocol::new(&blobs, None);
    let router = Router::builder(ep).accept(SYNC_ALPN, SyncHandler { peer: peer.clone(), heads: heads.clone(), pushed: Default::default() }).accept(iroh_blobs::ALPN, blobs_proto).spawn();
    // E35: "online" means a relay answered. With no internet that never happens, and the device
    // must still work on the local network, so wait at most 10 s (not at all in LAN-only mode).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(if lan_only() { 0 } else { 10 }), router.endpoint().online()).await;
    std::fs::write(dir.join("addr.json"), serde_json::to_vec(&router.endpoint().addr())?)?;
    if let Some(f) = follow_from {
        let (p, e) = (peer.clone(), router.endpoint().clone());
        tokio::spawn(async move {
            // E29: a closed lid, a lost signal or a sleeping teammate ends the connection. Keep
            // trying, waiting 1, 2, 4 ... 30 s between tries; each new watch says what we already
            // have, so everything missed arrives as one compact push.
            let mut wait = 1u64;
            loop {
                let started = Instant::now();
                match follow(p.clone(), e.clone(), f.clone()).await {
                    Ok(()) => eprintln!("[daemon] follow ended"),
                    Err(err) => eprintln!("[daemon] follow interrupted: {err:#}; retrying in {wait} s"),
                }
                if started.elapsed().as_secs() > 60 {
                    wait = 1; // it had been working: start the backoff again
                }
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                wait = (wait * 2).min(30);
            }
        });
    }
    // the app side: one JSON request per line on a local port
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("[daemon] online, app port {port}");
    loop {
        let (c, _) = l.accept().await?;
        let (peer, heads) = (peer.clone(), heads.clone());
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let (r, mut w) = c.into_split();
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let reply = (|| -> Result<serde_json::Value> {
                    let req: serde_json::Value = serde_json::from_str(&line)?;
                    let key = req["key"].as_str().context("key")?;
                    let field = req["field"].as_str().context("field")?;
                    let mut app = peer.app()?;
                    let item = app.photos.get_mut(key).context("no item")?;
                    let ptr = format!("/{}", field.replace('.', "/"));
                    if req["op"] == "get" {
                        return Ok(item.state.pointer(&ptr).cloned().unwrap_or(serde_json::Value::Null));
                    }
                    *item.state.pointer_mut(&ptr).context("no setting")? = req["value"].clone();
                    let mut repo = peer.repo()?;
                    let v = repo.save(&mut app, "edit")?;
                    peer.save_app(&app)?;
                    peer.set_head(repo.head)?;
                    if let Some(h) = repo.head {
                        let _ = heads.send(h);
                    }
                    Ok(serde_json::json!({"version": v.map(|v| v.short()), "ms": now_ms()}))
                })();
                let out = match reply {
                    Ok(v) => serde_json::json!({"ok": true, "result": v}),
                    Err(e) => serde_json::json!({"ok": false, "error": format!("{e:#}")}),
                };
                if w.write_all(format!("{out}\n").as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
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
            arg(&args, "--cert-days").map(|s| s.parse()).transpose()?.unwrap_or(1),
        )
        .await,
        "serve" => serve(&need_dir()?, relay_only).await,
        "pull" => pull(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), relay_only, args.iter().any(|a| a == "--closure"), args.iter().any(|a| a == "--compact")).await,
        "fetch-ranges" => {
            let ranges: Vec<(u64, u64)> = arg(&args, "--ranges").context("--ranges a-b,c-d")?.split(',').map(|r| {
                let (a, b) = r.split_once('-').context("range a-b")?;
                Ok((a.parse()?, b.parse()?))
            }).collect::<Result<_>>()?;
            fetch_ranges(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), &arg(&args, "--hash").context("--hash")?, &ranges, Path::new(&arg(&args, "--out").context("--out")?), relay_only).await
        }
        "fetch-media" => {
            let h = arg(&args, "--hash").context("--hash")?;
            if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("--hash must be 64 hex characters, got {h:?}");
            }
            fetch_media(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), &h, relay_only, arg(&args, "--out").map(PathBuf::from)).await
        }
        "render" => render(&need_dir()?, &arg(&args, "--key").context("--key")?, Path::new(&arg(&args, "--out").context("--out")?)).await,
        "import" => import(&need_dir()?, Path::new(&arg(&args, "--file").context("--file")?)).await,
        "daemon" => daemon(&need_dir()?, arg(&args, "--port").context("--port")?.parse()?, arg(&args, "--follow").map(PathBuf::from), relay_only).await,
        "fetch-swarm" => fetch_swarm(&need_dir()?, &arg(&args, "--from").context("--from a.json,b.json")?, &arg(&args, "--hash").context("--hash")?, arg(&args, "--stripe-mb").map(|n| n.parse()).transpose()?.unwrap_or(8), relay_only).await,
        "add-folder" => add_folder(&need_dir()?, Path::new(&arg(&args, "--folder").context("--folder")?)).await,
        "fetch-folder" => fetch_folder(&need_dir()?, Path::new(&arg(&args, "--from").context("--from")?), &arg(&args, "--hash").context("--hash")?, args.iter().any(|a| a == "--one-by-one"), relay_only).await,
        "export-bundle" => export_bundle(&need_dir()?, arg(&args, "--have"), &args_after(&args, "--media"), Path::new(&arg(&args, "--out").context("--out")?)).await,
        "seal-bundle" => {
            // seal for the given member devices (an app would take them from the members log)
            let to: Vec<ed25519_dalek::VerifyingKey> = arg(&args, "--to").context("--to cert.json,...")?.split(',').map(|c| {
                let cert: DeviceCert = serde_json::from_slice(&std::fs::read(c)?)?;
                kopano_core::identity::parse_key(&cert.body.device)
            }).collect::<Result<_>>()?;
            let input = arg(&args, "--file").context("--file")?;
            let out = arg(&args, "--out").context("--out")?;
            let mut w = std::io::BufWriter::new(std::fs::File::create(&out)?);
            let n = kopano_core::seal::seal(&mut std::io::BufReader::new(std::fs::File::open(&input)?), &mut w, &to)?;
            w.into_inner()?.sync_all()?;
            println!("{}", serde_json::json!({"event": "sealed", "recipients": to.len(), "bytes": n}));
            Ok(())
        }
        "import-bundle" => import_bundle(&need_dir()?, Path::new(&arg(&args, "--file").context("--file")?)).await,
        "add-media" => add_media(&need_dir()?, Path::new(&arg(&args, "--file").context("--file")?)).await,
        "edit" => edit(
            &need_dir()?,
            &arg(&args, "--key").context("--key")?,
            &arg(&args, "--field").context("--field")?,
            arg(&args, "--value").context("--value")?.parse()?,
            &arg(&args, "--message").unwrap_or_else(|| "Edit".into()),
        ),
        "status" => status(&need_dir()?),
        "protect-key" => {
            let name = keystore::protect(&need_dir()?)?;
            println!("{}", serde_json::json!({"protected": true, "entry": name}));
            Ok(())
        }
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
        "flood" => flood(Path::new(&arg(&args, "--from").context("--from")?), arg(&args, "--n").map(|n| n.parse()).transpose()?.unwrap_or(100), arg(&args, "--hold-s").map(|n| n.parse()).transpose()?.unwrap_or(20)).await,
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
