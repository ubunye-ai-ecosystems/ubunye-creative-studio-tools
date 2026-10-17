# B. Architecture decision report

Date: 2026-10-17. Inputs: baseline (01), technology assessment (`tech-assessment-raw.md`), experiments E1 to E8 (03, 04). Decisions marked **D-n**; each says what evidence supports it and what would make us change it.

## 1. The shape in one picture

```
                         CONTROL PLANE (hosted, small, no creative data)
   ┌──────────────────────────────────────────────────────────────────────────┐
   │ ThabangVision (Next.js + Supabase)          any other identity provider   │
   │  • "link this device" flow (browser approval)                             │
   │  • signs DEVICE CERTIFICATES: account ↔ device key, expiry                │
   │  • kopano_devices / kopano_projects / kopano_project_members (098)       │
   │  • optional: device presence + current relay URL (discovery hint)        │
   │  • publishes issuer keys at /.well-known/kopano-keys.json                │
   └───────────────▲──────────────────────────────────────────────────────────┘
                   │ once per device (link), then every ~7-30 days (renew)
                   │ peers verify certificates OFFLINE with the issuer key
   ════════════════╪════════════════════════════════════════════════════════════
                   │                 DATA PLANE (peer to peer)
   ┌───────────────┴────────┐   iroh 1.x QUIC, dialled by device key   ┌────────────────────────┐
   │ Device A (Johannesburg)│◄═══════ direct, hole-punched ═══════════►│ Device B (Nairobi)     │
   │                        │        ╲                          ╱      │                        │
   │ Ubunye Light/Photo/Film│         ╲  relay (ours or paid):  ╱       │ app + connector        │
   │   ▲ connector (JSON-RPC│          ╲ forwards encrypted    ╱        │ Kopano service         │
   │   │ over control port) │           ╲ packets, stores none╱         │  redb: objects, refs   │
   │ Kopano service         │            ═══════════════════            │  blob store: media     │
   │  redb: objects, refs   │                                           └────────────────────────┘
   │  blob store: media     │◄════ same protocol ════►┌───────────────────────────────┐
   └────────────────────────┘                         │ optional STUDIO PC (always on) │
                                                      │ a normal peer that keeps copies│
                                                      └───────────────────────────────┘
```

Boundaries stated precisely (the mission's rule against claiming "zero cloud"):
- **Hosted:** identity and membership (ThabangVision), certificate issuance, optional presence hints, and relays. Relays see device ids, IP addresses and who talks to whom, never content (QUIC/TLS end to end).
- **Never hosted:** creative files, edit histories, previews. They live on members' devices and, if the team wants, on a studio PC they own.

## 2. Layers and decisions

### Identity: account, device, certificate (D-1)
- **Decision:** one ed25519 key per device. The same key is the device's iroh endpoint id and the key that signs its versions, so "who is connecting" and "who signed this" are one fact. The identity provider (ThabangVision first) signs a short-lived **device certificate** binding account → device key. Peers verify offline with the provider's public key.
- **Evidence:** implemented and tested (`identity.rs`; conformance "a changed version is refused"; E5 step 7: a valid account that is not a member is refused at both the sync and the media protocol).
- **Rejected:** passing Supabase JWTs between peers (HS256 likely, 1-hour expiry, contains email, accepted by other apps on the shared project, issuer changes on rotation); OAuth device grant (Supabase has none); a separate Kopano account system (owner rule; no justification).
- **Open (Proposed):** the ThabangVision link flow and migration 098 (see roadmap phase 2). Revocation = short expiry + revocation list fetched when online. Offline devices keep what they already downloaded; we say so in the UI.

### Projects and permissions (D-2)
- **Decision:** a project starts from a signed genesis record listing trusted providers, members, roles (viewer, commenter, editor, admin) and lanes per editor. Every version is checked against it, and lane permission is **recomputed from the data**, never trusted from the version's own claim. In a merge, a change that equals one of the merged parents is accepted (that parent was verified); any new value needs the merger's lane permission.
- **Evidence:** implemented, tested in conformance and E5.
- **Proposed next:** membership changes as a signed, append-only membership log (Radicle's identity document idea), with admin signatures. Biscuit tokens (biscuit-auth 6, maintained, offline-verifiable, attenuable) are the candidate if we need delegatable capabilities; not needed for phase 1.

### History: signed versions over a manifest trie (D-3)
- **Decision:** Git's model, not Git's storage. Objects are content-addressed (BLAKE3). A version = parents + manifest root + touched lanes + message + author certificate, signed. The manifest is a canonical hash trie (≤ 64 entries per leaf, split by key-hash byte), so equal contents give equal roots on every peer.
- **Evidence (Measured, E4):** at 1,000,000 items a one-item edit writes 23 KB of new objects (flat list: 55 MB) and finding the change between two versions takes 0.12 ms (flat: 9 ms).
- **Rejected:** gitoxide/Git storage (no partial fetch of big files, no signing in gix, SHA-1/SHA-256 rather than BLAKE3 streaming); a flat manifest (measured 2,400× more bytes per edit at 1M items).

### Edit state and merging (D-4)
- **Decision:** item states are JSON owned by the app. Kopano merges three-way **per setting** only for kinds the connector declares mergeable; anything else that both sides changed becomes a **conflict record** in the project (lane `kopano.conflicts`) holding both versions. Kopano never merges by guessing and never drops either side.
- **Evidence:** implemented; conformance (different settings both survive; same setting = clash, both kept); E5 steps 9 to 11 between two processes.
- **Rejected for edit state:** CRDTs (Automerge, Loro). A creative setting changed by two people needs a human decision, not a silent last-writer-wins; CRDTs also keep growing history. **Kept as a candidate** for shared catalogue metadata (keywords, comments) if that needs live co-editing later; Loro is the strongest Rust option (tech assessment §4).

### Object storage (D-5)
- **Decision:** Kopano's small objects live in one embedded ACID database per device (redb), one transaction per save or pull; the head ref moves only after commit.
- **Evidence (Measured, E7):** 20,000 objects: 0.43 s vs 27 s (SSD) with one file per object; reads ~70× faster. Per-object durable commits on the HDD: 14 per second, so batching is mandatory.
- **Rejected:** loose files per object (Git-style) on Windows.

### Media (D-6)
- **Decision:** media never travels with versions. Items reference media by BLAKE3 hash and role (original, proxy, preview, export). Media moves only when a role asks, at the rung it needs, with verified, resumable range transfer (BLAKE3/bao, 16 KiB verification groups).
  - **Camera originals are immutable:** hash the whole file once (at import or first share). No chunking needed: identical files transfer 0 bytes under every method (E2).
  - **App documents that change in place** (layered images, project bundles): content-defined chunks (FastCDC, ~64 KiB average) stored as blobs. Measured: a 5% tile change in a 149 MB layered document sends 7.7% (whole-file and fixed blocks: 100%); a 4 KB metadata insertion in a RAW sends 0.14% (fixed blocks: 100%).
  - **Photo's `.pcraft` is already content-addressed by tile**: map tiles to blobs directly; CDC is unnecessary there.
  - **Re-encoded media** gains nothing from any method (100% in E2); do not pretend otherwise.
  - **Video for editors:** proxies by default; originals fetched only for the byte ranges a cut uses (Film already knows used time ranges and sample offsets; combining them is roadmap work).
- **Transport implementation:** iroh-blobs 0.103 works (E5: resumed a killed 553 MB transfer, fetching only the missing 295 to 346 MB), but its authors call it not production quality. **Decision:** keep it behind a Kopano `MediaStore` interface; before phase 3 either pin a reviewed version or implement the small request/response protocol directly on `bao-tree` (the tech assessment found this path credible).

### Transport and discovery (D-7)
- **Decision:** iroh 1.x for every connection (QUIC, TLS identity = device key, hole punching, relay fallback). Our own relays for production (iroh-relay is open source; n0 public relays are dev-only). Peers find each other by device key; the control plane can publish a presence hint (current relay URL) so dialling does not depend on n0's DNS.
- **Evidence (Measured, E5):** LAN path: connect 5 to 10 ms, small edit sync 1 to 10 ms, media 1.2 to 1.7 Gbit/s (same PC). Relay-only path via Frankfurt from Johannesburg: connect 1.2 s, three round trips 2.6 s.
- **Finding:** dialling an offline peer waits 30 s before failing. Presence must come from the control plane or a recent-contact cache, not from a timeout.

### Topology and the studio PC (D-8)
- **Decision:** any peer can serve any other (no master). A studio PC is just a peer that is always on and keeps full copies; it gives teams the "always available" property without cloud storage. Phase 1 sync is pull from known peers; gossip (iroh-gossip) only if measured fan-out needs it (E8).

### Connector contract (D-9)
- **Decision:** five calls (`info`, `snapshot`, `apply`, `describe`, plus a **change feed** to add in phase 1) with a contract version and declared kinds/lanes/mergeability. In-process it is a Rust trait; across processes the same calls go as JSON-RPC (the apps' control ports are the transport, so upstream files stay untouched). A connector may only write kinds it declares, so it cannot corrupt another app's items in a shared project. Seven conformance checks are implemented and pass for the lab's toy editor.

## 3. Alternatives considered as whole architectures

| Architecture | Verdict | Why |
|---|---|---|
| Central cloud repository (Frame.io / Blackmagic Cloud style) | rejected | owner rule: no hosting of creative files; recurring cost scales with media |
| Git + LFS / git-annex as the engine | rejected | LFS needs a server; Git storage has no verified partial fetch; Windows support of git-annex is weak; we still need signing and lanes on top |
| Radicle as the engine | rejected as dependency, adopted as design reference | code-only data model, no Windows installer, no large-file story (tech assessment §2) |
| CRDT document per project (Automerge/Loro for everything) | rejected for edit state | silent merges of creative decisions; history growth; still needs a media layer |
| Syncthing-style folder sync of app files | rejected | apps' own docs warn that two writers on one catalogue lose edits (Light `lock.rs`); no permissions, no history semantics |
| **Signed version DAG + manifest trie + per-setting merge + iroh transport + device certificates** | **chosen** | every property the mission lists is either measured, tested or has a bounded next step |

## 4. Threats and how the design answers them (summary)

| Threat | Answer | Status |
|---|---|---|
| Peer impersonation | iroh TLS binds connection to device key; certificate must name that key | Implemented, tested (E5 step 7) |
| Forged or altered version | ed25519 over canonical JSON; content addressing | Tested (conformance) |
| Writing a lane without permission | recomputed from data on every received version | Implemented, tested |
| Replay of an old head | an ancestor is "already had"; nothing rolls back | Tested (idempotence) |
| Withholding newer heads (freeze) | pull from several peers + control-plane head hint | Proposed |
| Equivocation (one device forks) | both forks are visible in the DAG and merge like any other | Proposed check |
| Damaged disk / bit rot | every read re-hashes; damaged objects removed and refetched | Tested (E5 step 12) |
| Malicious large frames / disk fill | frame cap 64 MiB, batch cap 10k; media gate allows only members; iroh-blobs ≥ 0.103.1 | Partly implemented; quotas Proposed |
| Lost device | revoke in control plane; certificate expiry; rotate project key if encryption at rest is added | Proposed |
| Metadata exposure | relay sees endpoint ids, IPs, timing; control plane sees membership | Documented, accepted |
