# E. Production roadmap

Small phases, each independently testable, each ending in something a person can use. Complexity is relative (S, M, L, XL), not a promise of dates; plan 08's week estimates were made before any measurement and are not repeated here.

The order follows the measured risks: identity and storage first (cheap, decisive), then one real app end to end, then distance, then the heavy media work.

## Phase 0 — Spec and lab hardening (S)
- **Objective:** freeze `kopano/1`: object encodings, version format, sync protocol frames, connector contract v1, conformance kit.
- **Minimum:** `SPEC.md` written from the lab code; property tests for the trie (canonical shape under random insert/delete orders) and merge (no change ever lost); fuzz the frame and tree decoders.
- **Tests:** existing 7 conformance checks + 6 core tests + new property/fuzz targets in CI (Windows, Linux, macOS).
- **Criteria:** fuzzers run 1 hour clean; spec and code agree (one reviewer reimplements object hashing from the spec alone).
- **Fails if:** a canonical-form ambiguity is found (two encodings for one value).
- **Risks:** none new.

## Phase 1 — Kopano service + Light, one person, many devices (M)
- **Objective:** the owner's own use case: edit photos on two of his own computers, history, restore, no account needed.
- **Minimum:** `kopano` service (one per device; owns the redb database; tray or background), CLI; Light connector (from the lab) talking to the running app's control port with a token; change feed from Light's `catalog.log` instead of full scans; "History" and "Restore" in a small panel (web view or egui in the fork's `ubunye/` overlay).
- **Dependencies:** Light control port token (small patch in the fork, listed in `patched-paths.txt`), or run Kopano as the only client of a non-default port.
- **Tests:** conformance against the real app; restore round trip on a real 24 MP RAW library; crash during save (kill -9) never loses the last good version.
- **Criteria:** small-edit save ≤ 50 ms at 10k photos (today 264 ms with full scans); first sync of 10k photos ≤ 2 s on LAN (today 0.32 s transfer + process start).
- **Fails if:** the change feed misses edits (compare against full scan in a soak test).
- **Risks:** Light upstream changes the control protocol (mitigation: pin by upstream version; conformance in fork CI).

## Phase 2 — ThabangVision identity (M)
- **Objective:** sign in once per device with a ThabangVision account; teammates are real accounts.
- **Minimum:** migration 098 (`kopano_devices`, `kopano_projects`, `kopano_project_members`, flag `kopano` off), `POST /api/kopano/link/start`, `/link` approval page, `/api/kopano/link/token`, a dedicated Kopano issuer key (not the Supabase or service-role key), `/.well-known/kopano-keys.json`, certificate expiry 7 to 30 days + revocation list.
- **Dependencies:** the open ThabangVision items that touch auth: JWT secret rotation (TASK-081), shared-auth risk R-1. Kopano does not depend on Supabase JWTs, so it is not blocked by them, but suspension must revoke devices.
- **Tests:** RLS coverage test passes; link flow e2e (Playwright) on dev; a revoked device is refused by peers after its certificate expires or after the revocation list is fetched.
- **Criteria:** no client insert into device tables; no email in certificates; the desktop never receives a Supabase token.
- **Fails if:** any path lets a client register a device for another account.
- **Risks:** production auth changes are review items for the owner (repository rule: present before production adoption).

## Phase 3 — Two people, two networks (L)
- **Objective:** Johannesburg and Nairobi (or two home networks) on one Light project.
- **Minimum:** our own iroh relay (iroh-relay, open source) in at least one region near users; presence hints in the control plane (no 30 s dial timeouts); invites; lane permissions in the UI; conflict review screen ("keep mine / keep theirs / keep both as versions").
- **Media:** keep iroh-blobs behind a `MediaStore` interface; audit and pin a version, or implement the request/response on `bao-tree` directly.
- **Tests:** real two-machine runs on separate networks (and one on a mobile hotspot behind carrier NAT); packet loss injected on one side; kill mid-transfer; relay outage.
- **Criteria:** small edit visible on the other side in under 5 s on relay, under 1 s direct; zero lost edits across 1,000 randomized concurrent-edit trials.
- **Fails if:** relay traffic per active user per month exceeds the agreed budget (measure, then decide relay plan).
- **Risks:** hole-punch rate on SA mobile networks unknown (unverified); relay cost.

## Phase 4 — Studio PC and many members (M)
- **Objective:** an always-on peer and teams of 5 to 25.
- **Minimum:** "studio hub" mode (headless service, keeps full copies), location log ("who holds which original") with a minimum-copies rule before anyone may delete a local original; gossip of head announcements only if fan-out measurements require it.
- **Tests:** E8 fan-out repeated on real machines; hub offline and back; a member offline for a month returns.
- **Criteria:** time for an edit to reach every online member grows no worse than linearly with members.

## Phase 5 — Film (XL)
- **Objective:** editor on proxies, colourist on used segments, export matches.
- **Minimum:** stable clip ids (replace the shared `next_id`), media ids by BLAKE3, lanes edit/grade/sound split (grade pinned to source timecode), used-range → byte-range mapping from Film's demuxers, proxy sharing.
- **Tests:** conformance on Film; byte-identical CPU export from originals after a round trip through two peers.
- **Risks:** upstream churn in Film (200+ commits per sync); Film changes may need to be upstreamed.

## Phase 6 — Photo (L)
- **Objective:** retouch versions that cost only the changed tiles.
- **Minimum:** connector via folder bundles; tiles as blobs (no chunking needed: already BLAKE3 per tile); web persistence later.
- **Tests:** conformance; a one-brush-stroke edit transfers only the touched tiles (measure).

## Phase 7 — Hardening and launch gate (M)
- Security review (external if possible), threat model update, quotas, encryption at rest for the redb file and blob store if the owner wants it, protocol upgrade test (v1 ↔ v2 peers), docs for third-party connector authors, feature flag rollout.
