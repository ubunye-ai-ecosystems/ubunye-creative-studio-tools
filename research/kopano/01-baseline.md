# A. Existing architecture assessment (baseline)

Date: 2026-10-17. Method: code reading of the forks on `develop`, the ThabangVision repo on `origin/main`, and the installed Ubunye Light 0.5.0. Every claim below has a file reference in the agent reports kept with this folder, or was observed by running the software. Nothing here was taken from documentation alone.

Status words used in this folder: **Implemented** (code exists), **Tested** (automated check passes), **Measured** (a number from a real run on this PC), **Proposed** (design, not built), **Unverified** (believed, not checked).

## 1. What exists today

### Kopano itself
Nothing existed before today except the plan `08-collaboration-plan.md` (12 October, design only). No code, no spec, no tests. The draft `kopano/` folder started this morning (Light-specific) was discarded at the owner's request: Kopano must not be built on top of one app.

### The three apps (Rust + egui, desktop + wasm)

| Capability | Ubunye Light | Ubunye Photo | Ubunye Film |
|---|---|---|---|
| What a user edits | per-photo develop settings (JSON), ratings, flags, metadata | layered document: structured layers (JSON) + pixel tiles | timeline (JSON); media never modified |
| File format | `catalog.snap` (JSON) + `catalog.log` (JSON lines, CRC-32 per record), atomic writes | `.pcraft`: ZIP or folder; `manifest.json` + `tiles/<blake3>.zst` (256 px tiles, sparse, BLAKE3 of uncompressed bytes) | `.fcproj`: one JSON envelope, schema v14, atomic writes |
| Media identity | `content_hash`: SipHash-1-3 128-bit (not cryptographic) | BLAKE3 for tiles; originals never hashed | 64-bit non-crypto fingerprint of size + first/last MiB (optional) |
| Local history | 200 history steps + 20 auto versions per photo, persisted | 50 undo states in memory; crash autosave persisted every 10 min | 200 undo in memory; 20 autosaves every 5 min; crash journal |
| Outside control | control port (JSON lines, loopback, **no auth**): read every photo, apply settings | control port (loopback, **token auth**), `photocraft-cli serve`, MCP | control port (loopback, **no auth**), MCP, CLI |
| Change notification | none; the op log can be tailed | none; poll `revision` | none; poll `revision` |
| Web build storage | OPFS | none (save = download) | OPFS, one recovery slot + media copies up to 4 GiB each |
| Networking | none | none | none (model download only) |

Consequence for Kopano: every app can be connected **without editing upstream files** through its existing control channel, which keeps the daily upstream sync safe. None can push change events, so connectors poll (Photo, Film) or tail the op log (Light).

### ThabangVision (identity and control plane)
- Sign-in: Google or email + password, browser cookie sessions only (`@supabase/ssr`). No route accepts a Bearer token. No devices, API keys, OAuth apps or desktop sign-in exist.
- The installed Supabase auth library supports an OAuth 2.1 server (off in config) but **no device authorization grant**; local JWT checks only for RS256/ES256; production signing is probably HS256 (unverified). The legacy JWT secret is recorded as leaked and not yet rotated (TASK-081).
- The Supabase projects are shared with other apps (open P0 risk R-1): a Supabase user id alone does not prove someone is a ThabangVision person.
- `public.projects` is already taken (migration 097). Next free migration number: 098. Migrations are applied by hand with a test and rollback script, and an RLS coverage test fails the build on unsafe policies.
- `/tools` downloads promise "no account needed", so Kopano sign-in must stay optional for solo use.

## 2. What can be reused

| Need | Reuse | Evidence |
|---|---|---|
| App integration | each app's control port + headless CLI | agent audits; Light probed live today |
| Change feed (Light) | `catalog.log` op journal | `crates/catalog/src/journal.rs` |
| Content addressing (Photo) | BLAKE3 tile names map 1:1 to Kopano blobs | `crates/format/src/store.rs:43-45` |
| Proxies, export from originals (Film) | complete on desktop | `media_pool.rs:312-313` |
| Used time ranges of a cut (Film) | `project_manager::used_ranges` + demuxer byte offsets | not yet combined into byte ranges |
| Notes/comments | Photo Note annotations, Film markers with comments | plan 08 said "none": wrong |
| Embedded database | redb (already in iroh-blobs' dependency tree) | measured E7 |
| Transport | iroh 1.3 (stable since 1.0, 2026-06-15) | ran E5 |
| Verified media transfer | iroh-blobs 0.103.1 (**not production quality** per its README) | ran E5; risk R2 |
| Device sign-in | a ThabangVision "link this device" flow issuing signed device certificates | identity audit; matches the prototype's model |

## 3. Genuine capability gaps
1. No collaboration protocol, spec or conformance tests (now prototyped in `lab/`).
2. No device identity anywhere in ThabangVision; no non-browser sign-in.
3. No cryptographic hash of originals in Light or Film (SipHash / partial fingerprint only). A connector must compute BLAKE3 at import.
4. Film: one shared id counter (`next_id`) for every object, so two people adding clips would collide; grades are stored per timeline clip, not per source timecode. Film needs stable ids before concurrent editing.
5. Photo: no web persistence; tile store private (reachable only through folder bundles).
6. No app emits change events.
7. No relay, no discovery service, no "who holds which file" record.

## 4. Technical risks found
- **R1 Unauthenticated control ports** (Light, Film): any local process can drive the app. A connector must use a non-default port and should be paired with a token (Photo already has one).
- **R2 iroh-blobs is pre-production** and had a disk-fill bug fixed in 0.103.1 (2026-10-06). Pin it behind our own interface or replace it with a small protocol on `bao-tree`.
- **R3 iroh public relays are for development only** (rate-limited, no SLA). Production needs our own relays or a paid plan.
- **R4 Shared Supabase auth (R-1)** and the unrotated JWT secret: never accept a Supabase JWT as a peer credential.
- **R5 Windows small-file cost**: one file per object is about 1 ms per object (measured E7). The object store must be a database or pack files.
- **R6 Full-scan saves**: a connector that snapshots every item on every save is O(catalogue size) (6.3 s at 100k items before the store fix). The contract needs a change feed.

## 5. Incorrect or outdated assumptions in plan 08 (12 October)

| Plan 08 said | Reality |
|---|---|
| Photo history is memory only | undo is in memory, but a crash autosave is persisted |
| Photo hook is a `Services` trait | it is a struct of optional closures |
| "expose" Photo's tile store | it is private; use folder bundles (which delete unreferenced tiles on save) |
| Film hook `MediaPool::resolve` | the function is `source_for` |
| Film browser storage "up to 4 GiB" | 4 GiB is a per-file media copy cap; projects are not stored |
| Comments/approvals: none in any app | Photo has Notes, Film has markers with comments |
| Only Photo has a History panel | Film has one too |
| Lanes per role in Film (edit/grade/sound) | today grade and sound live inside the same clip record; lanes need Film changes |
| iroh in the browser "to be proven" | iroh works in browsers through relays only (no hole punching): every byte relayed |
| Mailbox: R2 (cloud) recommended | owner decision 2026-10-17: no cloud hosting of creative files; a studio PC can be the always-on peer |
| Identity: ed25519 per person | owner decision: use ThabangVision accounts; keys are per **device**, vouched for by the account provider |
