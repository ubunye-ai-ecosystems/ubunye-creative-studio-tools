# 08 · Ubunye Projects: collaboration and version history plan

Status: **parked as a long-term to-do** (owner, 2026-10-12). Proposal only; nothing here is built yet. Tracked as Milestone 8 in [07-implementation-plan.md](07-implementation-plan.md). Working name for the protocol: **Kopano** (Sesotho for "gathering"). The product name in the apps stays "Ubunye Projects".

## 1. The goal in one scenario

Naledi in Johannesburg shoots a wedding: 800 RAW photos (about 24 GB) and 50 GB of video.

- She opens Ubunye Light and Ubunye Film on her laptop and presses **Share**.
- Wanjiru, a retoucher in Nairobi, opens the link in her browser. She sees every photo within minutes, picks up the 20 the client starred, and retouches them.
- Sipho, an editor in Durban, cuts the film from small proxies on his desktop. He never downloads the 50 GB.
- Lerato, a colourist, grades only the seconds that are in the cut, at full quality.
- The client watches in the browser, leaves notes on exact frames, and presses **Approve**.
- Every save is a version. Anyone can see who changed what, compare, and restore. Nothing is ever overwritten silently.
- The final export is made from the originals and matches what everyone approved.

All of this runs on the machines people already own. The only possible bill is optional cloud storage beyond the free tier.

## 2. The idea: move decisions, not pixels

Footage and RAW files never change. What people create on top of them is tiny: a grade is a few KB of numbers, a cut is a list of timecodes, a note is a sentence pinned to a frame. So Kopano syncs decisions all the time and moves media only when a role needs it, at the quality that role needs, and only the part it needs.

This is the Git idea applied to media:

| Git | Kopano | What the creator sees |
|---|---|---|
| Files named by hash | Every object named by its BLAKE3 hash | Nothing (it just never duplicates) |
| Commit | Version: a signed note listing each lane's state and its parent | **Save** / automatic checkpoints |
| Log | Version graph | **History** panel with thumbnails |
| Checkout | Restore = new version equal to an old one | **Restore** (never destructive) |
| Push / pull / fetch | Exchange missing hashes with peers | Happens in the background |
| Partial clone (git-annex) | A version can mention an original without holding it | Collaborators work without the 50 GB |
| Files in a tree | **Lanes** (see below) | Roles |
| Merge | Automatic across lanes; never inside a lane | "Two versions, keep which?" only in rare offline cases |
| Branches, staging, rebase | Not exposed | Nothing |

## 3. Lanes: why people can work at the same time

Code merges are painful because two people edit the same file. Media teams already split work by role, so Kopano splits a project into lanes and each lane has one writer at a time.

| App | Lanes | Lock granularity |
|---|---|---|
| Light (photos) | selects (picks, stars, flags), develop, notes | **per photo**: two people can develop different photos at once |
| Photo (retouch) | document, notes | per document |
| Film (video) | edit, grade, sound, notes | per lane per sequence |

Rules:

1. Different lanes or different photos never conflict. Their versions combine automatically, the way Git combines changes to different files.
2. One writer per lane (or per photo). The app shows "Lerato is grading". Locks are leases that expire, so a crashed laptop does not block the team forever.
3. Notes and approvals only ever add, so they need no lock and never conflict.
4. If two people changed the same lane while offline, both versions are kept and a person picks. Kopano never merges by guessing.
5. Grades are pinned to source media plus source timecode, not to timeline position (the Resolve "remote grade" idea). Re-trimming a clip keeps its grade.

We deliberately do **not** do live co-editing of one timeline. Blender Mixer tried live co-editing of one scene and its docs warn it can corrupt data. One writer per lane is simpler and safe.

## 4. The media ladder

The creator's own device makes every rung, so there is no cloud processing.

| Rung | 50 GB video (estimate) | 800 RAW photos (estimate) | Who needs it |
|---|---|---|---|
| Thumbnails | MBs | ~20 MB | everyone |
| Browser preview | 1 to 2 GB (H.264) | ~250 MB (JPEG) | director, client, reviewer |
| Editing proxy | 2 to 5 GB | not needed | editor |
| Originals, used parts only | ~5 GB if the cut uses a tenth | the 20 selected RAWs, ~600 MB | colourist, retoucher |
| Full originals | 50 GB | 24 GB | stay with the owner; final export |

Why it matters between Johannesburg and Nairobi, at 20 Mbps upload (estimate): 50 GB takes about 5.5 hours, a 2 GB proxy about 13 minutes, a version about one second.

"Used parts only" works because BLAKE3 lets anyone fetch any byte range of a huge file and verify it without the rest (iroh-blobs implements this; about 6% overhead). Film already has its own demuxers, so it can say which byte ranges hold the frames in the cut.

Estimates in this section are not measured yet. Phase 0 measures them on our own footage.

## 5. How data travels (near free)

```
  Naledi's laptop  <--- iroh (QUIC, direct, hole-punched) --->  Sipho's desktop
        |                                                            |
        |            (browsers cannot connect directly)              |
        v                                                            v
   [ relay ]  <------- Wanjiru's browser (wasm app + OPFS) ------->  [ relay ]
        |
        v  optional, for when Naledi's laptop is off (load-shedding, travel)
   [ mailbox: R2 bucket or a "studio hub" PC ]  holds encrypted versions + proxies
```

1. **Peer to peer, desktop to desktop:** iroh. Free.
2. **Browser:** connects through a relay. iroh's makers run free public relays. We can self-host one later.
3. **Mailbox (optional, recommended for shared projects):** because the uploader's laptop will often be off. Either an old PC at the studio running the same peer software, or the R2 bucket we already have (10 GB free, no download fees, then about USD 0.015 per GB per month). It only ever stores encrypted objects. Proxies and versions go there by default; originals only when the owner chooses.

Storage backends are swappable through OpenDAL (disk, browser OPFS, R2/S3, NAS).

## 6. Privacy and permissions without a server

- Every person has a key pair (ed25519). It is their signature on versions, like a signed commit.
- An invite is a permission slip signed by the owner: "Wanjiru may write the retouch lane of this project". Peers check the signature before accepting her versions.
- Everything stored or relayed is encrypted with a project key. A mailbox or relay sees only noise. A hash alone gets a stranger nothing.
- Removing someone rotates the project key for everything after that point. What they already downloaded stays with them; we say so in the UI.
- Later, a ThabangVision login can vouch for a key, so a booking's client becomes a reviewer automatically. Payments are never touched.

## 7. What we reuse and what we build

Audit facts are from reading the three forks on 2026-10-12.

| Need | Already in our forks | Reuse from open source | We build |
|---|---|---|---|
| Non-destructive edits | Light (JSON settings), Film (JSON timeline), Photo (adjustment layers; brush work is pixels) | | |
| Local history | Light: 200 steps + 20 auto versions per photo; Film: 20 auto-saves + crash journal; Photo: undo in memory only | | persistent named versions in Photo and Film |
| Content hashing | Photo: BLAKE3 tiles; Light: weak SipHash; Film: partial 64-bit | BLAKE3 | full BLAKE3 for every original |
| Proxies + export from originals | Film, complete | | share proxies, pin to original hash |
| Deterministic export | Film (tested), Light (same backend) | | |
| Timeline interchange | Film: OTIO, EDL, FCPXML, AAF | OpenTimelineIO | |
| Browser storage | Light: OPFS; Film: OPFS up to 4 GiB; Photo: none | | Photo web persistence |
| Transfer + verified ranges | none | iroh, iroh-blobs | |
| Storage backends | none | OpenDAL | |
| Compression | Photo uses zstd | zstd | |
| Signing, encryption | none | ed25519-dalek, chacha20poly1305 (audited RustCrypto) | key + invite format |
| Comments/approvals | none | (optional) Automerge | simple add-only note log |
| Versions, lanes, locks, adapters | none | | **the Kopano core library** |
| UI: History, Share, Review | Photo has a History panel | | egui panels shared by all three |

Our own code is: the spec, one Rust library (`kopano`), three adapters, and shared egui panels. Everything hard below that is reused.

## 8. Plugging into the apps

All three apps are Rust and build for desktop and browser, so the shared piece is one Rust crate, not a TypeScript SDK.

Each app gives Kopano an adapter with four jobs:

```rust
trait ProjectAdapter {
    fn snapshot(&self) -> LaneStates;            // my state, split into lanes, plus media used
    fn load(&mut self, state: &LaneStates);       // load a version
    fn describe(&self, a: &LaneStates, b: &LaneStates) -> Vec<String>; // "Exposure +0.3"
    fn thumbnail(&self, state: &LaneStates) -> Image;
}
```

| App | Hook point (already exists) | Adapter work |
|---|---|---|
| Light | `catalog::Store` trait, `LibraryStores`, `Session::commit`; op log is already JSON lines | map per-photo settings and versions to lanes; strong hash at import |
| Photo | `ui-egui::Services`, `Session::edit`, `.pcraft` BLAKE3 tiles | expose tile store; keep objects for history; add OPFS persistence on web |
| Film | `Services` trait, `MediaPool::resolve`, `Session::edit` | split `.fcproj` into edit/grade/sound lanes; stable clip ids; media id by hash instead of path; range fetch for used segments |

Fork rule stays: add files, do not edit upstream ones. Each unavoidable hook line goes in `ubunye/patched-paths.txt`. The daily upstream sync keeps working.

## 9. "Reverse engineering": what we study, and the line we do not cross

The upstream projects follow a clean-room rule against Adobe, and so do we. We study behaviour and public formats; we never copy or decompile proprietary code.

| Study | How | Why |
|---|---|---|
| Lightroom XMP (`crs:`) | public spec; Light already reads and writes it | photographers can bring edits in and out |
| Resolve collaboration | use it as a user (free Blackmagic Cloud ID): roles, bin/timeline locks, remote grades | copy proven UX, not code |
| Premiere Team Projects / Frame.io | use the products; read public docs | review UX, frame-pinned comments |
| OpenTimelineIO | open spec + library | let Resolve/Premiere users exchange timelines with Film |
| git-annex | open source; read its design docs | location tracking, "keep at least two copies" |
| Ink & Switch Patchwork | public lab notes | universal version control UX, lessons |
| Our own forks | read code, run them, measure | the real data models (section 7) |

## 10. Plan, timeline and exit tests

Estimates assume me (Claude) as the main engineer working autonomously on feature branches, the owner reviewing PRs and making product calls, and two or three real creatives for testing. Upstream churn (200+ commits a sync) can slow phases 4 and 5.

| Phase | Work | Time | Done when |
|---|---|---|---|
| 0. Spec + measure | write the Kopano spec; benchmark proxies (H.264/VP9/AV1), BLAKE3 hashing, range fetch, iroh across two networks, using our own footage on T: | 1 week | numbers replace the estimates in section 4 |
| 1. Core + Light, offline | `kopano` crate (objects, versions, lanes, history); Light adapter; History panel | 2 weeks | save, history, compare, restore in Light, survives crash |
| 2. Two desktops | iroh sync, keys, invites, per-photo locks, mailbox on R2 | 2 to 3 weeks | two PCs on different networks co-edit one shoot; kill the network mid-transfer and it resumes |
| 3. Browser | Light web joins over relay; Review mode (comment on photo, approve) | 2 weeks | a browser in another country reviews and edits |
| 4. Film | lanes, stable clip ids, hash media ids, shared proxies, used-segment fetch, export from originals | 3 to 4 weeks | editor on proxies + colourist on used segments; master export matches; no 50 GB download |
| 5. Photo | web persistence, tile objects as history, retouch lane | 2 weeks | retouch versions cost only changed tiles |
| 6. Harden | security review, threat model, two-country real test, docs, feature flags | 2 weeks | owner signs off on a real job |

**First useful result (Light, two people, two countries): about 5 to 6 weeks. All three apps: about 14 to 16 weeks.**

## 11. Skills the work needs

| Skill | Used for |
|---|---|
| Rust (systems level) | core library, adapters |
| WebAssembly + browser APIs (OPFS, WebCodecs, WebTransport) | browser builds |
| Networking (QUIC, NAT traversal, relays) | iroh integration, testing across countries |
| Applied cryptography (using audited libraries correctly) | keys, invites, encryption, rotation |
| Video engineering (codecs, GOPs, timecode, colour) | proxies, used-segment fetch, export correctness |
| Photo pipeline (RAW, colour spaces, tiles) | Light and Photo adapters |
| egui UI + UX for artists | History, Share, Review panels |
| Fork maintenance | keeping hooks small across daily upstream syncs |
| Security review | before any public launch |
| Real creatives | the only proof that matters is a real job |

## 12. Cost

| Item | Cost |
|---|---|
| All software listed | free, open source (MIT/Apache; Syncthing not needed) |
| Peer to peer transfer | free |
| Relays | free public relays; self-host later on an existing machine |
| Mailbox | R2: first 10 GB free, then ~USD 0.015/GB/month, no download fees; or a studio PC at no extra cost |
| Example: 100 creators, proxies + versions only, 20 GB each | ~2 TB ≈ USD 30/month (estimate) |
| Same, originals uploaded too | ~USD 750 per 1,000 × 50 GB projects per month (estimate); default is originals stay local |

## 13. Risks

- **Browser limits:** Film web is single-threaded and caps imported media at 4 GiB; proxy creation must happen on desktop.
- **Upstream churn:** hooks must stay tiny or merges will hurt.
- **Upstream CI on `develop` is already red;** fix before adding features.
- **iroh in the browser** must be proven in phase 0, not assumed.
- **Encryption vs deduplication:** per-project keys mean the same footage in two projects is stored twice. Accepted for privacy.
- **Revocation** cannot recall files already downloaded.

## 14. Decisions for the owner

1. Protocol name: keep "Kopano" or choose another.
2. Mailbox default: R2 (cheap, always on) or studio PC (free, depends on power).
3. Originals in the cloud: off by default (recommended), or allowed per project.
4. Start order: Light first (recommended), then Film, then Photo.
5. Who are the first two or three real creatives to test with.

## 15. Prior art (checked 2026-10-12)

- Commercial: DaVinci Resolve + Blackmagic Cloud (project library, roles, optional media sync); Premiere Team Projects (cloud-dependent; an AWS outage disconnected users in Oct 2025); Frame.io; Anchorpoint and Diversion (Git-style for artists, commercial).
- Open source: git-annex/DataLad (partial content, location tracking); Oxen (Rust data VCS; S3 backends unfinished); Xet (Hugging Face, content-defined chunking); SnowFS (GPL-3, inactive since 2023); Blender Mixer and Multi-User (live co-editing, unmaintained or unstable); Kitsu (review server); Suture (OTIO timeline diff, one author, early).
- Research: Ink & Switch Patchwork (universal version control on Automerge; research system for documents).
- Plumbing we build on: iroh / iroh-blobs (BLAKE3 verified range fetch), p2panda (pre-1.0), OpenDAL, zstd.

Nobody combines an open protocol, role lanes, a media ladder with verified partial fetch, and browser plus desktop editors that speak it natively, without a central server. That is the gap.
