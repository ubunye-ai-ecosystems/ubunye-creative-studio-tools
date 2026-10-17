# Kopano technology assessment (raw research)

Date of research: 2026-10-17. Researcher: Claude (web research only, no code written, no repos changed).

How to read this file:

- **[V]** = verified on the cited URL during this session (2026-10-17).
- **[U]** = unverified: from memory, a third-party summary, or an inference. Treat as a lead, not a fact.
- **Judgement** sections are my opinion, clearly separated from facts.
- Scores in the matrix at the end are judgement, built on the facts above them.

Kopano needs (from the brief): signed Git-like history, lanes and roles, lazy fetch of large media, resumable verified transfer, NAT traversal with relay fallback, Windows first-class, offline then reconcile, catalogues of 100k+ assets, Rust + egui on Windows/macOS/Linux and wasm. Only accounts, device keys and membership live in Supabase.

---

## 0. Headline findings (read this first)

1. **iroh core is 1.0 and stable.** iroh 1.0.0 shipped 2026-06-15; latest is 1.3.0 (2026-09-28). Wire protocol and API are declared stable; wire-breaking changes only in a major release. [V] https://crates.io/api/v1/crates/iroh , https://www.iroh.computer/blog/v1
2. **iroh-blobs is NOT 1.0 and its own README says the current line is not production quality.** Current is 0.103.1 (2026-10-06, built on iroh ^1.0.0). The README says "this version of iroh-blobs is not yet considered production quality" and "if you need production quality, use iroh-blobs 0.35". [V] https://docs.rs/iroh-blobs/latest/iroh_blobs/ , https://crates.io/api/v1/crates/iroh-blobs/0.103.1/dependencies
3. **That production fallback is a trap.** iroh-blobs 0.35.0 depends on `iroh ^0.35` [V] https://crates.io/api/v1/crates/iroh-blobs/0.35.0/dependencies , and n0 says 0.35 gets no further releases and its public relays run only until 2026-12-31 [V] https://www.iroh.computer/blog/v1 . So there is no "production-blessed iroh-blobs on iroh 1.x" today. Kopano must either own the blob layer (on `bao-tree`) or accept iroh-blobs 0.10x as a pinned, wrapped dependency.
4. **iroh-blobs had a real security bug this month.** Any peer that could connect to a provider could push data into its store (disk fill) in 0.94.0 to 0.103.0; fixed in 0.103.1 (2026-10-06) with backports, broken versions yanked. [V] https://www.iroh.computer/blog/iroh-blobs-push-fix
5. **iroh public relays are for development only.** Rate-limited, no SLA, "free to use for development and testing". Production means self-hosting iroh-relay or paying (Pro USD 19/month, 5 MB/s, 100 GB egress). Kopano moves multi-GB media, so relayed fallback traffic must go to relays we run. [V] https://docs.iroh.computer/about/faq , https://www.iroh.computer/blog/shared-relays
6. **Browser/wasm is relay-only for iroh.** No hole punching from a browser; all browser traffic flows through a relay. [V] https://docs.iroh.computer/deployment/wasm-browser-support . For multi-GB video this means a wasm client cannot be a peer for bulk media without relay cost.
7. **Radicle is a useful design reference, not a library to embed.** Its storage is Git (namespaces per peer, signed refs), COBs are commit DAGs. Official installer supports Linux and macOS only; Windows CLI exists per third-party news. [V/U mixed, see section 2]
8. **gitoxide cannot be Kopano's store.** No commit signing, no push, no LFS, SHA-256 not done. [V] https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md
9. **Keyhive, Subduction, Willow Rust, p2panda, Lix, NextGraph are all pre-1.0 / alpha.** Good ideas to borrow; none safe as a core dependency in 2026.
10. **Spacedrive v2 (Apache-2.0 since 2026-03-24) is rebuilding on iroh with BLAKE3 content identities**, beta planned 2026-11-01. Closest real-world peer to Kopano's shape and worth watching. [V] https://github.com/spacedriveapp/spacedrive

---

## 1. iroh family (n0-computer)

### 1.1 iroh (endpoint, relays, discovery)

Facts:

- Latest 1.3.0 on 2026-09-28; 1.2.0 2026-09-09; 1.1.0 2026-08-25; 1.0.3 2026-07-20; 1.0.2 2026-07-06; 1.0.1 2026-06-29; 1.0.0 2026-06-15; rc.0 2026-05-07. Roughly one minor every 2 to 3 weeks. 3.35M downloads. Licence MIT OR Apache-2.0. [V] https://crates.io/api/v1/crates/iroh
- 1.0 promise: "an iroh v1 endpoint will be able to communicate with another iroh v1 endpoint, regardless of minor version or language"; "Any change that affects the wire stability of iroh will always coincide with a major release." Official bindings: Python, Node.js, Swift, Kotlin. [V] https://www.iroh.computer/blog/v1
- Support schedule: 0.35 no further releases, its public relays run until 2026-12-31; 0.9x and 1.0.0-rc relays stopped 2026-09-30. [V] https://www.iroh.computer/blog/v1
- Usage claim: "more than 200 million endpoints created" on public relays in 30 days; "It's normal to see 95% of data transferred in a connection pass directly between devices." (vendor claim, not a benchmark). [V] https://www.iroh.computer/blog/v1
- Repo: 12.7k stars, 774 forks, 2,637 commits; contributor count not captured. [V] https://github.com/n0-computer/iroh
- CI tests Linux (amd64), macOS (ARM64), Windows (`x86_64-pc-windows-msvc`), each with all/none/default features. Android and wasm32 not in that workflow. [V] https://raw.githubusercontent.com/n0-computer/iroh/main/.github/workflows/tests.yaml
- Crypto: QUIC with TLS 1.3, certificates tied to Ed25519 endpoint IDs, planned move to raw public keys (RFC 7250); 0-RTT forward-secrecy caveats noted. [V] https://docs.iroh.computer/about/faq . Post-quantum key exchange "already available in 1.0" [V] https://www.iroh.computer/blog/the-road-to-iroh-1-0 (note: the FAQ says "not post-quantum secure"; the docs disagree with each other, treat as unresolved).
- No security audit mentioned in FAQ or README. [V absence] https://docs.iroh.computer/about/faq
- 1.0.2 fixed a pre-auth relay DoS: a short datagram frame panicked the relay process (`panic = "abort"`), fixed with fuzzing added. Clients not affected. [V] https://www.iroh.computer/blog/iroh-1-0-2

Relays:

- What relays see: before a direct path exists, which endpoint IDs talk and how many bytes flow; IDs and IP addresses of connected endpoints; which endpoints are paired. Payloads are end-to-end encrypted. Relays store nothing; n0 says it does not record this data. [V] https://docs.iroh.computer/about/faq
- Public relays: n0-run in US West, US East, Europe Central, Singapore; "free to use for development and testing"; rate-limited, limits "can change at any time"; no SLA. [V] https://docs.iroh.computer/about/faq , https://docs.iroh.computer/concepts/relays
- Paid: Community USD 0 (variable limit); Pro USD 19/month (shared relay, 5 MB/s, auth, 100 GB egress, USD 0.09/GB over, 10k concurrent connections); Dedicated USD 199/region on top of Pro (no rate limit, custom DNS, 250 GB egress, 60k connections). [V] https://www.iroh.computer/blog/shared-relays
- Self-hosting: open source, needs a public IP and DNS name, built-in ACME TLS; access control by endpoint-ID allowlist, shared token, or HTTP callback to your service. [V] https://docs.iroh.computer/about/faq . 1.0 added bearer-token access control and Let's Encrypt multi-hostname. [V] https://github.com/n0-computer/iroh/releases
- Ports: TCP 443 (and 80 for ACME), UDP 7842 for QUIC address discovery. [U] community draft + iroh firewall checklist, https://git.coopcloud.tech/decentral1se/iroh-relay/pulls/2/files , https://docs.iroh.computer/configuring-networks

Discovery:

- Default: Pkarr records published over HTTP to an `iroh-dns-server` (default n0's `dns.iroh.link`), resolved via DNS TXT `_iroh.<z32-id>.<origin>`; record carries home relay URL and optionally socket addresses, signed by the endpoint key. [V] https://docs.iroh.computer/concepts/discovery
- mDNS local discovery is a separate crate; Mainline DHT discovery is opt-in. [V] https://docs.iroh.computer/concepts/discovery , https://docs.iroh.computer/about/faq
- Privacy: the default DNS server learns your endpoint ID and relay URL; lookups reveal who is being looked up. [V inference stated on page] https://docs.iroh.computer/concepts/discovery

wasm / browser:

- `iroh` builds to wasm with `default-features = false`; "Browser support is a relay-only subset"; no UDP so no hole punching; WebTransport/WebRTC paths "not implemented"; no official npm package; iroh-gossip works in browsers since 0.33; "still expanding it to its other protocols". [V] https://docs.iroh.computer/deployment/wasm-browser-support

Benchmarks:

- No official published throughput numbers found. `iroh-netbench` 0.1.0 and an in-repo bench exist; perf.iroh.computer returned no readable data. Third-party "gigabit-class" claims (instagit.com) are auto-generated and **unverified**. [V absence] https://docs.rs/crate/iroh-netbench/0.1.0 , https://perf.iroh.computer

Funding / company: N0, INC. holds copyright [V repo footer]. Venture funding amount [U, not checked].

Judgement:

- Best-in-class fit for Kopano transport: dial by public key, hole punching with relay fallback, QUIC streams, Windows in CI, a stability promise since June 2026.
- Real costs: you must run relays (at least two regions, per n0's own advice) and probably your own `iroh-dns-server`, or skip DNS discovery entirely and publish endpoint ID + relay URL through Supabase (the control plane already knows the devices). This is simpler and leaks less.
- Release cadence is fast (minor every ~2-3 weeks). Pin minors and upgrade deliberately.

### 1.2 iroh-blobs

Facts:

- Highest 0.103.1 (2026-10-06); depends on `iroh ^1.0.0`, `bao-tree ^0.16`, optional `redb ^4.1.0`. 827k downloads. MIT OR Apache-2.0. [V] https://crates.io/api/v1/crates/iroh-blobs , https://crates.io/api/v1/crates/iroh-blobs/0.103.1/dependencies
- Not 1.0. README and docs: "this version of iroh-blobs is not yet considered production quality ... use iroh-blobs 0.35". [V] https://raw.githubusercontent.com/n0-computer/iroh-blobs/main/README.md , https://docs.rs/iroh-blobs/latest/iroh_blobs/
- 0.35.0 pins `iroh ^0.35` and `redb =2.4`. [V] https://crates.io/api/v1/crates/iroh-blobs/0.35.0/dependencies
- Security fix (push not enforced, disk-fill) 0.94.0 to 0.103.0, fixed 0.103.1 + backports; note `get_many` and `observe` now have their own permission modes, so an allowlist on `get` alone was bypassable. [V] https://www.iroh.computer/blog/iroh-blobs-push-fix
- Protocol: request/response over QUIC; request whole blobs, byte ranges, or whole HashSeq sequences in one request; links are 32-byte BLAKE3 hashes. [V] README above
- Stores: `store::mem` and `store::fs` (feature `fs-store`, uses redb + reflink-copy). [V] https://docs.rs/iroh-blobs/latest/iroh_blobs/
- Store design (blog, 2025-03-12): small blobs inline in redb (16 KiB default threshold), large blobs as plain files `<hash>.data` + `<hash>.obao4`; outboard stored only down to 16 KiB chunk groups so verification granularity is at most 16 KiB; partial blobs always on disk with a bitfield; chunks only go absent to present so multi-peer downloads combine safely; can serve partial blobs; bitfield held in memory until periodic sync; startup re-validation of giant files "could take minutes", hence bitfield persisted; deletion is whole-blob only via tag-based GC. [V] https://www.iroh.computer/blog/blob-store-design-challenges
- 0.95 (2025-10-13): connection pool, multi-provider downloader, provider event masks (intercept/deny requests), stream traits for compression; "only BLAKE3 is supported", chunk group size hardcoded. [V] https://www.iroh.computer/blog/iroh-blobs-0-95-new-features
- `bao-tree` 0.16.1 (2026-08-26): "BLAKE3 verified streaming with custom chunk groups and range set queries", MIT OR Apache-2.0. [V] https://crates.io/api/v1/crates/bao-tree
- Repo: 164 stars, 52 forks, 70 open issues. [V] https://github.com/n0-computer/iroh-blobs
- wasm: not documented for blobs; docs say browser support is being extended beyond core + gossip. [V absence] https://docs.iroh.computer/deployment/wasm-browser-support

Memory on multi-GB files:

- Published facts only say data is a plain file and the bitfield is held in memory until sync [V blog above]. No measured RAM numbers published. [V absence]
- Judgement (arithmetic, not measured): with 16 KiB chunk groups the outboard holds one 64-byte parent node per pair of groups, roughly 0.4% of the data size (about 40 MB for a 10 GB file), on disk not in RAM. Memory should be bounded by buffers + bitfield. Must be measured before relying on it.

Judgement:

- The design is exactly what Kopano needs (verified streaming, range requests, partial blobs, resumable, multi-source). The packaging is not: no stable API, a recent auth bug, and its blessed version is tied to an end-of-life iroh.
- Recommendation: build Kopano's blob layer on `bao-tree` + iroh 1.x streams with our own small, versioned wire protocol and our own store (or vendor/fork iroh-blobs 0.103.1 behind a `kopano-blobs` trait). Either way, Kopano owns the permission check on every request type.

### 1.3 iroh-gossip

- 0.101.0 (2026-06-15), MIT/Apache-2.0, 875k downloads. [V] https://crates.io/api/v1/crates/iroh-gossip
- HyParView + PlumTree epidemic broadcast; 32-byte topic IDs; sans-IO `proto` module. [V] https://github.com/n0-computer/iroh-gossip
- Works in browser since 0.33. [V] https://docs.iroh.computer/deployment/wasm-browser-support
- Judgement: fine for "new head available" announcements inside a project; not needed for small teams where direct sync to known members is simpler.

### 1.4 iroh-docs and iroh-willow

- iroh-docs 0.101.0 (2026-06-15), 197k downloads. [V] https://crates.io/api/v1/crates/iroh-docs
- Model: replica (namespace key = write capability) of entries keyed by (key, author); value is BLAKE3 hash + size + timestamp; range-based set reconciliation (arXiv:2212.13567); redb store; depends on iroh-blobs + iroh-gossip. [V] https://github.com/n0-computer/iroh-docs
- Judgement: last-writer-wins per key, no history DAG, so it is not a version system. Its reconciliation idea is worth copying for "which blobs does the peer have".
- iroh-willow: crate 0.0.1 (2025-02-07) "minimal implementation of Willow, Meadowcap, and Confidential Sync"; no releases. [V] https://crates.io/api/v1/crates?q=willow , https://github.com/n0-computer/iroh-willow . Judgement: dormant/experimental, do not depend.

---

## 2. Radicle (heartwood)

Facts:

- Latest Radicle 1.10.3 (2026-09-08); MIT and Apache-2.0. [V] https://radicle.dev/ , https://radicle.dev/2026/09/08/radicle-1.10.3.html
- Site says it works on "Linux, macOS and BSD variants, as well as Windows". [V] https://radicle.dev/ . But the official install script only supports macOS (x86_64, aarch64) and Linux (x86_64, aarch64 musl) and exits on anything else; 1.10.3 checksummed builds are Linux + macOS only. [V] https://radicle.dev/install , release page above. Windows CLI "without WSL" since 1.3.0 (Aug 2025) and "Windows build support" in 1.6.0 (Jan 2026) per AlternativeTo headlines. [U] https://alternativeto.net/news/2025/8/radicle-1-3-0-introduces-canonical-references-and-windows-cli-support
- Identity: Ed25519 node key shared as a DID (`did:key`); no permission needed to create. Repo identity doc (JSON) under `refs/rad/id` with delegates and signature threshold; RID derived from the first version. Signed refs: node signs all its refs on every change (`refs/rad/sigrefs`). [V] https://radicle.dev/guides/protocol
- COBs: stored under `refs/cobs/<type>`; each change is a signed Git commit in a DAG; sync = union of commit graphs ("non-destructive, idempotent"); state = reduce in topological order with a deterministic merge or a flagged conflict. [V] same
- Replication: gossip of node, inventory and ref announcements; data moves via Git fetch, multiplexed over one connection; seeding policies choose repos; bootstrap nodes. Private repos are not encrypted at rest. [V] same
- Storage: plain Git repos, per-peer namespaces, shared object DB. No large-file story mentioned. [V] same
- `radicle-cob` crate 0.21.1 (2026-07-24), 41k downloads, MIT OR Apache-2.0. [V] https://crates.io/api/v1/crates/radicle-cob . Whether it uses git2 (libgit2) or gix underneath: [U] (docs.rs fetch failed). 1.10.3 bumped `gix-packetline`, so gix is at least partly used. [V] release page
- Own transport, not iroh. [U] (protocol guide's transport-encryption section is currently removed "because of a security vulnerability" per the fetched page; I could not read details). [V that section is missing]

Judgement:

- Reusable as **design**: did:key identity, signed identity document with delegate threshold, sigrefs, COB-as-signed-DAG with deterministic reducer. These map directly to Kopano lanes/roles and history.
- Not reusable as **library** for Kopano: Git object storage (SHA-1, no partial fetch of big blobs), its own network stack, Windows not in official installs, and `radicle-cob` is tied to Git storage.

---

## 3. Git internals and large-file lessons

### 3.1 gitoxide (gix)

- 0.89.0 (2026-10-08), monthly-ish releases (0.83 Apr to 0.89 Oct 2026), 52M downloads, MIT OR Apache-2.0, still 0.x. [V] https://crates.io/api/v1/crates/gix
- Status: object writing mostly done; ref transactions mostly done; commit creation done; **commit signing missing/unclear**; tree merge partial; blob 3-way merge mostly done; **push missing**; fetch/clone mostly done; **SHA-256 missing**; **LFS missing**. [V, inferred from wording because checkboxes did not survive extraction] https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md
- Judgement: good for an optional "export to Git" bridge or reading Git repos; wrong as Kopano's core store.

### 3.2 Jujutsu (jj-lib) and Pijul (for ideas)

- jj-lib 0.46.0 (2026-10-07), Apache-2.0, "experimental version control system", monthly releases. [V] https://crates.io/api/v1/crates/jj-lib
- Judgement: borrow jj's **operation log** (every repo mutation recorded, undoable, concurrent ops merged) and "conflicts are first-class objects you can commit" idea. These suit offline-then-reconcile far better than Git's "stop and resolve now".
- Pijul: patch-theory DVCS, 1.0 beta; partial clones. [V] https://pijul.org/ . Licence GPL-2.0 [U]. Judgement: elegant for text, no large-binary story, licence unsuitable to embed in permissive products.

### 3.3 Git LFS lessons

- Pointer file: `version`, `oid sha256:<hex>`, `size`; under 1024 bytes. [V] https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md
- Locking API: `/locks`, `/locks/verify`, `/locks/:id/unlock`; one lock per file; only the owner (or `force`) unlocks; others' locks block pushes. [V] https://github.com/git-lfs/git-lfs/blob/main/docs/api/locking.md
- Judgement / lessons: (a) keep history tiny by storing pointers, not bytes; (b) locks matter for unmergeable binaries (a PSD-like layered file) and a lock must be an advisory, signed, expiring record in a P2P world, not a server mutex; (c) LFS's failure mode is the central server: Kopano replaces it with peers + availability tracking.

### 3.4 git-annex lessons

- Content in `.git/annex/objects/` keyed by backend keys like `SHA256E-s<size>--<hash>.<ext>`; a separate `git-annex` branch holds per-key location logs (`timestamp present|absent repo-UUID`), merged by line union, newest timestamp wins; `numcopies.log`. [V] https://git-annex.branchable.com/internals/
- Judgement: this is the single most relevant prior art for Kopano's "who has which media" layer. Copy: a union-mergeable, append-only availability log per asset keyed by device; a "minimum copies" policy before a device may drop its local original. Avoid: the extension-in-key trick (same bytes, two keys).

---

## 4. CRDT frameworks

### 4.1 Automerge

- Rust crate `automerge` 0.12.0 (2026-09-16), monthly releases, MIT, 721k downloads. (JS "Automerge 3.x" is built on this Rust core; crate numbering differs.) [V] https://crates.io/api/v1/crates/automerge
- Automerge 3.0 (July 2025): memory cut "by over 10x"; "pasting Moby Dick into an Automerge 2 document consumes 700Mb of memory, in Automerge 3 it only consumes 1.3Mb"; a long-history doc that "hadn't loaded after 17 hours" loads "in 9 seconds"; same file format as 2.x. [V] https://automerge.org/blog/automerge-3/
- Sync protocol assumes a reliable in-order stream between two peers, keeps per-peer `State` (their heads), uses Bloom filters, loops until neither side has news; based on arXiv 2012.00472. [V] https://docs.rs/automerge/latest/automerge/sync/index.html
- `samod` (Rust automerge-repo, compatible with JS automerge-repo) 0.15.0 (2026-09-30), MIT. [V] https://crates.io/api/v1/crates/samod
- Benchmark (dmonad crdt-benchmarks, automerge 2.1.10, i5-8400, Node 20.5): B4 real-world trace 259,778 ops: time 14,326 ms, doc 129,116 bytes, parse 1,805 ms; B4x100 skipped. [V] https://github.com/dmonad/crdt-benchmarks (pre-3.0 numbers, so pessimistic).
- 1M-op memory figure: **no published number found**. [V absence]

### 4.2 Loro

- `loro` 1.16.2 (2026-09-21), MIT, 900k downloads; frequent releases (1.12 Apr to 1.16 Sep 2026). [V] https://crates.io/api/v1/crates/loro
- Features: movable tree, movable list, LWW map, rich text (Fugue), version control / time travel, shallow snapshots ("like a Git shallow clone"); Rust, JS (wasm), Swift; ~6.2k stars; GitHub Sponsors funded. [V] https://github.com/loro-dev/loro . 1.0 date: Oct 2024 [U].
- Benchmarks: dmonad suite (loro 0.10.1): B4 time 3,089 ms, doc 258,228 bytes, parse 13 ms; B4x100 time 309,689 ms, doc 25.8 MB, parse 1,304 ms. [V] https://github.com/dmonad/crdt-benchmarks . Loro native Rust bench (Oct 2024, M2 Max): apply 88 ms vs Automerge 451 ms; decode 0.19 ms vs 506 ms; Yrs faster on concurrent list inserts (13.95 ms vs 130.63 ms). [U: from search snippet of https://loro.dev/docs/performance/native ; page returned 403 to direct fetch]. Loro's own docs say their JS numbers are from Nov 2023 and "somewhat outdated". [U snippet, https://www.loro.dev/docs/performance]

### 4.3 Yrs (Yjs Rust port)

- `yrs` 0.28.0 (2026-09-17), MIT, 4.16M downloads, still 0.x. [V] https://crates.io/api/v1/crates/yrs
- Benchmarks: ywasm 0.9.3 B4 time 28,675 ms, doc 159,929 bytes, parse 16 ms. [V] https://github.com/dmonad/crdt-benchmarks
- Judgement: excellent for text editors; Yjs garbage-collects history by default, which fights Kopano's "every version is signed and kept" goal.

### 4.4 Are CRDTs right for creative edit state? (Judgement)

- Creative edit state is mostly **parameters** (exposure, white balance, crop), **ordered stacks** (layers, adjustment stack, effects), and **timelines** (clips with in/out points). Users think in versions and choices, not character streams.
- CRDT per-field LWW silently picks a winner when two people move the same slider offline. For a colourist that is data loss of intent. A lane model ("my grade" vs "your grade") already implies branches and explicit merges.
- Recommended split:
  - **Edit state per asset or per sequence: immutable signed snapshots in a DAG + schema-aware 3-way merge on structured data** (JSON-like, per-field, with app-provided merge rules and a conflict record when both sides changed the same field). Deterministic, small, explainable, easy to sign.
  - **Shared catalogue metadata (collections tree, ratings, tags, comments, picks, presence): CRDT**. Loro fits best (movable tree for folders/collections, LWW maps for ratings, time travel for audit, shallow snapshots to bound history). Shard by collection or by asset-ID prefix, not one doc for 100k+ assets; per-asset small docs or a sharded set keeps sync and memory bounded.
  - Live co-editing of one timeline in real time is the only place a list/tree CRDT is worth it for edit state; treat it as a later feature.

---

## 5. Keyhive / Beelay / Subduction (Ink & Switch)

- Keyhive: "pre-alpha", "DO NOT use this release in production applications", not security audited; Apache-2.0; components keyhive_core, keyhive_crypto, BeeKEM (concurrent TreeKEM), keyhive_wasm; 260 stars; no published releases on GitHub. README says auth-enabled sync moved to **Subduction**, Beelay's successor. [V] https://github.com/inkandswitch/keyhive
- Notebook: pre-alpha opened 2025-03-10; latest entry 2026-07-31 (BeeKEM security analysis guest post). [V] https://www.inkandswitch.com/keyhive/notebook/
- Subduction: "Sync protocol for hash-linked data"; early preview, "very unstable API", not for production; Apache-2.0 / MIT; Rust 1.91+, wasm for browser/Node; transports WebSocket, HTTP long-poll, **iroh (QUIC)**; Sedimentree (encrypted hierarchical fragments partitioned by leading-zero bytes of hashes); `subduction_keyhive` integration; 122 stars. [V] https://github.com/inkandswitch/subduction
- Judgement: the right research direction (local-first E2EE groups + capabilities + sync of ciphertext). Not usable in 2026. Design Kopano's permission records so a Keyhive-style group could replace them later.

---

## 6. Lix (opral)

- "Embeddable version control system for files and tables"; MIT; "Lix is in alpha"; repo is ~32 MB Rust vs 1.5 MB TypeScript (Rust core); JS SDK on npm `@lix-js/sdk`, Rust quickstart, Python/Go planned; storage backends memory, filesystem, OPFS, S3; plugins map parts of a file to SQL rows (paragraph, cell, property) so only Markdown and CSV get structured diff/merge, others whole-file history; 769 stars; last push 2026-10-14. [V] https://github.com/opral/lix , https://api.github.com/repos/opral/lix/languages , https://api.github.com/repos/opral/lix
- Judgement: the plugin-per-format idea ("app describes how its document splits into addressable entities") is exactly what Kopano needs for app-neutral merges. Lix itself is alpha, aimed at text/tables, and has no P2P or large-binary story. Borrow the plugin contract, do not depend.

---

## 7. Other local-first stacks

### 7.1 p2panda

- `p2panda-core` 0.7.1 (2026-08-21), MIT OR Apache-2.0, 21.6k downloads. [V] https://crates.io/api/v1/crates/p2panda-core
- Crates: core, net, sync, blobs, store, auth (group permissions), encryption (post-compromise security), spaces, discovery, stream; uses iroh, BLAKE3, Ed25519, CBOR; "not yet considered stable for production use", stability at v1.0.0; EU NGI funded; 580 stars. [V] https://github.com/p2panda/p2panda
- Judgement: closest full-stack match in philosophy and it is on iroh. Small community, NGI grant-funded (funding cliff risk), pre-1.0. Worth reading `p2panda-auth` and `p2panda-encryption` designs.

### 7.2 Willow / Meadowcap / Earthstar

- Willow: data model with destructive editing (prefix pruning), Meadowcap capabilities (owned and communal namespaces), Confidential Sync, Drop Format for sneakernet; NLnet NGI Assure funded. [V] https://willowprotocol.org/
- Rust: `willow25` 0.7.9 (2026-08-27), `willow-data-model` 0.8.0, `meadowcap` 0.6.0 (2026-08-10); repo on Codeberg (worm-blossom/willow_rs); the old GitHub repo was archived 2025-10-23 and listed WGPS/sync as "Eventually". [V] https://crates.io/api/v1/crates?q=willow , https://github.com/earthstar-project/willow-rs
- Judgement: Meadowcap is a clean offline-verifiable capability model, but tied to Willow's data model; Willow's destructive-editing semantics clash with "keep signed history". Watch, do not adopt.

### 7.3 NextGraph

- Decentralised, encrypted, local-first platform; Apache-2.0 / MIT; "available in alpha release (with limited features)", SDK preview alpha; uses Yjs (y-crdt), Automerge, RDF/Oxigraph; NLnet funded. [V] https://nextgraph.org/ . Written in Rust [U].
- Judgement: platform, not a library; alpha; RDF-centric. Not a fit.

### 7.4 Syncthing BEP (design reference)

- Blocks 128 KiB to 16 MiB in powers of two, constant per file; choose the smallest size giving fewer than 2000 blocks (128 KiB up to 250 MiB files, 16 MiB at 16 GiB+); Index (full) and Index Update (incremental) messages; per-file version vectors keyed by the first 64 bits of the device ID. [V] https://docs.syncthing.net/specs/bep-v1.html . Go, MPL-2.0 [U].
- Judgement: proven at scale for whole-folder sync. Copy: version vectors for per-asset availability, incremental index updates. Kopano's BLAKE3 tree (16 KiB groups) gives finer verification than BEP's fixed blocks.

### 7.5 Spacedrive v2 (reference implementation to watch)

- v2 is a Rust rewrite; peers via iroh/QUIC with relay fallback; BLAKE3 content identities with sampled hashes for duplicate candidates then full verification; Apache-2.0 (AGPL-3.0 before 2026-03-24); 2.0 beta planned 2026-11-01; Windows buildable from source, nightly binaries only macOS ARM64 + Linux x86-64; ~39k stars. [V] https://github.com/spacedriveapp/spacedrive
- Judgement: same transport bet as Kopano; its sampled-hash trick is useful for fast import of 100k-asset libraries (sample first, full BLAKE3 lazily).

---

## 8. Large-file chunking and storage

### 8.1 Xet / xet-core (Hugging Face)

- Rust client library behind HF Xet storage; crates hf-xet, xet-client, xet-data, xet-core-structures, xet-runtime; Apache-2.0; wasm build in CI; README says "not meant to be used directly", talks to HF CAS backend; whether CAS is open/self-hostable is not stated; 611 stars. [V] https://github.com/huggingface/xet-core
- Chunking: content-defined, target 64 KB (8 to 128 KB range); chunks grouped into xorbs of ~64 MiB; shards reference xorbs. [V via search of HF docs] https://huggingface.co/docs/xet/deduplication , https://huggingface.co/docs/xet/upload-protocol
- Published results (2025-02-12): gemma-2-9b-it-GGUF 191 GB stored as 97 GB in 1,515 blocks; upload at 50 MB/s 509 min to 258 min ("nearly 2x"). [V] https://huggingface.co/blog/from-chunks-to-blocks
- Judgement: proof that CDC + block aggregation works for multi-GB binaries. Copy the design (CDC chunks, aggregate into large blocks for storage efficiency, shard index), not the client (bound to HF's server).

### 8.2 fastcdc crate

- 5.0.0 (2026-08-22), MIT, 2.33M downloads, "FastCDC (content defined chunking) in pure Rust". [V] https://crates.io/api/v1/crates/fastcdc
- Judgement: the default choice if Kopano chunks mutable large documents. Do **not** CDC immutable camera originals (RAW, camera video): whole-file BLAKE3 + bao ranges already give partial fetch and verification, and they never change.

### 8.3 Oxen

- Rust data version control for large datasets; Apache-2.0; needs `oxen-server` (Linux/macOS/Docker; not Windows) or OxenHub; CLI supports Windows; block-level dedup only in an `experiments/` folder; ~1.2k stars; marketing claims, no formal benchmarks. [V] https://github.com/Oxen-AI/Oxen
- Judgement: client-server, not P2P. Not a fit beyond ideas on metadata extraction.

---

## 9. Capability security and identity

### 9.1 Biscuit (biscuit-auth, Eclipse Biscuit)

- `biscuit-auth` 6.0.0 (2025-07-16), Apache-2.0, 12.4M downloads; no release since July 2025. [V] https://crates.io/api/v1/crates/biscuit-auth
- Public-key signed tokens; offline attenuation (holder can add restricting blocks, never widen); Datalog policies; wasm components; Rust and Haskell examples. [V] https://doc.biscuitsec.org/getting-started/introduction.html
- Spec: Ed25519 default, secp256r1 (ECDSA SHA-256) optional; revocation ID per block = its signature; third-party blocks (signed by an outside party, needs Datalog 3.2+); Datalog versions 3.0 to 3.3. [V] https://github.com/eclipse-biscuit/biscuit/blob/main/SPECIFICATIONS.md
- Judgement: best fit for Kopano permissions today. Supabase (control plane) mints an authority block per member ("member M has role R in project P, lanes L, until T"); the device attenuates for a peer session; any peer verifies offline with the project's public key. Revocation needs a gossiped revocation list keyed by revocation ID, plus short expiry.

### 9.2 UCAN

- Spec 1.0.0 (DAG-CBOR envelope, Varsig, Ed25519/P-256/secp256k1, did:key; delegation + invocation required, revocation recommended). [V] https://github.com/ucan-wg/spec . A 2026 overview calls core 1.0 still an RC. [U] search snippet.
- Rust: `ucan` crate stuck at 0.7.0-alpha.1 / 0.4.0, last updated 2023-06-28, pre-1.0 format. [V] https://crates.io/api/v1/crates/ucan
- Judgement: good model, poor Rust implementation state. Avoid.

### 9.3 Meadowcap

- See 7.2. `meadowcap` 0.6.0 (2026-08-10). [V] Judgement: tied to Willow paths/timestamps; not general enough for Kopano.

### 9.4 Identity (judgement)

- Device key = Ed25519 (same curve as iroh endpoint IDs, Radicle did:key, Biscuit default). Keep the iroh endpoint key separate from the signing key used for history, so rotating a transport key does not orphan signed history; bind them with a signed statement.
- Membership: Supabase is the issuer of record online; mirror it as a signed, append-only membership log inside each project (Radicle identity-doc pattern with delegates + threshold) so offline peers can verify who could sign what at the time.

---

## 10. Things released or moving in 2025-2026 that matter

- iroh 1.0 (2026-06-15) and paid relays (Sept 2026). [V]
- Automerge 3.0 memory fix (July 2025); samod 0.15 (Sept 2026). [V]
- Subduction (Keyhive's sync, iroh transport) early preview. [V]
- Spacedrive v2 on iroh, Apache-2.0 since 2026-03-24, beta due 2026-11-01. [V]
- Lix Rust core rewrite (alpha, active Oct 2026). [V]
- Xet storage on the HF Hub (CDC at scale, 2025). [V]
- Teamtype (Rust P2P collaborative editing of local text files, updated May 2026). [U] search aggregator only.
- No dedicated open-source "local-first multi-GB media collaboration protocol" found that is production-ready. That gap is Kopano's opening. [V absence across searches]

---

## 11. Decision matrix (judgement)

Criteria: 1 Architectural fit, 2 Production maturity, 3 Rust compatibility, 4 Cross-platform (Windows + wasm), 5 Performance evidence, 6 Security model, 7 Maintenance/dependency risk (5 = low risk), 8 Licence (5 = permissive, embeddable), 9 Operational simplicity (5 = no servers), 10 Extensibility. Scores 1 to 5.

| Candidate | 1 Fit | 2 Maturity | 3 Rust | 4 Platforms | 5 Perf evidence | 6 Security | 7 Maint. risk | 8 Licence | 9 Ops | 10 Extensible |
|---|---|---|---|---|---|---|---|---|---|---|
| iroh 1.x (endpoint) | 5 dial-by-key, NAT, relay fallback | 4 1.0 stable since Jun 2026, fast minors | 5 native | 4 Windows in CI; browser relay-only | 2 no official published benchmarks | 4 E2EE QUIC/TLS1.3, no audit, relay DoS fixed | 4 company-backed, 12.7k stars | 5 MIT/Apache | 3 must run relays (+ DNS or own discovery) | 5 custom ALPN protocols |
| iroh-relay (self-host) | 5 needed for big relayed media | 4 versioned with iroh 1.x | 5 | 4 Linux server; clients all | 2 none published | 3 sees IPs and pairings; pre-auth DoS fixed 1.0.2 | 4 | 5 | 2 public IP, DNS, TLS, 2 regions | 4 token/allowlist/callback auth |
| iroh-blobs 0.103 | 5 verified ranges, partial, multi-source | 2 README: "not production quality" | 5 | 3 no browser story | 2 design notes only | 3 push bug Oct 2026, now fixed | 3 small repo, 0.x churn | 5 | 4 embedded | 4 event masks, stream traits |
| bao-tree 0.16 | 5 the core primitive | 3 0.x but narrow scope | 5 | 5 pure Rust | 2 none published | 5 BLAKE3 verified streaming | 4 | 5 | 5 | 4 |
| iroh-gossip 0.101 | 3 head announcements only | 3 0.x | 5 | 5 works in browser | 1 none | 3 | 4 | 5 | 5 | 3 |
| iroh-docs 0.101 | 2 LWW KV, no history | 2 | 5 | 3 | 1 | 3 | 3 | 5 | 5 | 2 |
| Radicle (heartwood) | 3 great ideas, Git storage, no big files | 4 1.10.x, regular releases | 4 Rust but Git-bound | 2 official installs Linux/macOS only | 1 none | 4 signed refs, did:key, delegates | 3 small team | 5 | 3 seed nodes | 2 COB types only inside Git |
| gitoxide (gix) | 2 Git model, SHA-1, no LFS | 3 0.89, no push/signing | 5 | 5 | 3 known fast, not checked here | 3 | 4 active | 5 | 5 | 3 |
| jj-lib (ideas) | 3 op log, first-class conflicts | 2 "experimental" | 5 | 4 | 1 | 3 | 4 | 5 | 5 | 3 |
| git-annex / LFS (design) | 4 availability log, locks, pointers | 5 decades of use | 1 not Rust libs | 3 | 1 | 3 | 5 | 2 annex AGPL/GPL [U]; LFS MIT [U] | 2 LFS needs server | 3 |
| Automerge (rs 0.12) | 4 metadata yes, edit state partly | 4 widely used, 3.0 memory fix | 5 Rust core | 5 wasm first-class | 3 published but pre-3.0 | 3 no auth built in | 4 Ink & Switch | 5 MIT | 5 | 4 samod repo |
| Loro 1.16 | 5 tree, maps, versions, shallow snapshots | 4 1.x, frequent releases | 5 Rust core | 5 wasm + Swift | 4 published benches, best parse times | 3 no auth built in | 3 small core team, sponsors | 5 MIT | 5 | 4 |
| Yrs 0.28 | 3 text-first, GC drops history | 4 huge usage, still 0.x | 5 | 5 | 3 slower in B4 via wasm | 3 | 4 | 5 MIT | 5 | 3 |
| Keyhive / Subduction | 4 exactly the right problem | 1 pre-alpha, unaudited | 5 | 4 wasm, iroh transport | 1 | 2 promising, unaudited | 2 research lab | 5 | 4 | 3 |
| Lix | 3 plugin-per-format idea | 1 alpha | 4 Rust core, JS-first SDK | 4 OPFS, FS | 1 | 2 | 3 | 5 MIT | 4 | 4 plugins |
| p2panda | 4 local-first + iroh + groups | 2 0.7, not production | 5 | 3 | 1 | 4 PCS group encryption | 2 grant-funded | 5 | 4 | 4 modular |
| Willow / Meadowcap (willow25) | 2 destructive edits vs history | 2 0.x, sync "eventually" | 5 | 3 | 1 | 4 Meadowcap | 2 tiny team, NLnet | 5 | 4 | 3 |
| NextGraph | 2 RDF platform | 1 alpha | 4 [U] | 3 | 1 | 4 E2EE | 2 | 5 | 2 brokers | 2 |
| Syncthing BEP (design) | 4 proven folder sync | 5 mature | 1 Go | 5 | 3 in the wild | 4 TLS device certs | 5 | 4 MPL [U] | 3 relays/discovery | 2 |
| xet-core (design) | 4 CDC at scale | 4 in production at HF | 4 Rust, bound to HF CAS | 4 wasm in CI | 4 published 2x numbers | 3 | 4 | 5 Apache | 1 needs CAS server | 2 |
| fastcdc 5.0 | 4 for mutable big docs | 4 mature, 2.3M downloads | 5 | 5 pure Rust | 3 paper + usage | n/a 3 | 4 | 5 MIT | 5 | 3 |
| Oxen | 2 client-server | 3 | 5 | 2 server not on Windows | 1 marketing only | 3 | 3 | 5 Apache | 2 oxen-server | 3 |
| Biscuit 6.0 | 5 offline attenuable roles | 4 6.0 stable, Eclipse | 5 | 5 wasm | 3 | 5 signed, Datalog, revocation IDs | 3 slow release cadence | 5 Apache | 5 | 5 Datalog + third-party blocks |
| UCAN | 4 model fits | 2 spec 1.0/RC | 1 Rust crate stale since 2023 | 3 | 1 | 4 | 2 | 5 | 5 | 4 |
| Spacedrive v2 (reference) | 3 same transport bet, file manager | 1 pre-beta | 5 | 3 | 1 | 3 | 3 | 5 Apache since Mar 2026 | 4 | 2 app, not lib |

---

## 12. Top recommendation per layer (judgement)

1. **Transport: iroh 1.x.** Self-host `iroh-relay` in at least two regions (Johannesburg/Cape Town-near cloud + Europe) with token or callback auth against Supabase; skip n0 public relays in production. Publish each device's endpoint ID + home relay through Supabase (control plane) instead of public pkarr/DNS, and enable mDNS for same-LAN studios. Browser/wasm clients are relay-only: limit them to metadata, previews and proxies, not originals.
2. **Content transfer and storage: BLAKE3 content addressing with `bao-tree` verified streaming.** Own a small versioned `kopano-blobs` protocol and store (or a pinned fork of iroh-blobs 0.103.1 behind a trait), with permission checks on every request type. Whole-file BLAKE3 for immutable originals (RAW, camera video), range requests for lazy fetch and scrubbing, persisted bitfields for resume, git-annex-style union-merged availability log + min-copies rule. Use `fastcdc` (~64 KiB average, Xet-style) only for large mutable app documents. Generate and sync small proxies/previews first.
3. **History and versions: Kopano's own signed Merkle DAG**, not Git storage. Commits are small manifests (asset hashes + edit-state snapshot hashes + parents), signed with Ed25519 device keys; per-lane signed refs (Radicle sigrefs pattern); a jj-style operation log for undo and offline reconciliation; first-class recorded conflicts. Optional `gix` export bridge later.
4. **Collaborative metadata: Loro** for catalogue-level shared state (collections tree, ratings, tags, comments), sharded per collection so 100k+ assets never live in one doc. Edit state (develop settings, layer stacks, timelines) uses immutable snapshots + schema-aware 3-way merge via a Lix-style per-app plugin contract, not a CRDT. Automerge 3 (+ samod) is the fallback if Loro's small team becomes a risk.
5. **Permissions and identity: Ed25519 device keys + Biscuit tokens.** Supabase mints role/lane tokens; devices attenuate per session; peers verify offline; revocation IDs gossiped and expiry kept short. Mirror membership as a signed, threshold-governed identity log inside each project (Radicle pattern) so history stays verifiable offline. Track Keyhive/Subduction for E2EE groups later; avoid UCAN (no maintained Rust crate) and Meadowcap (tied to Willow).

Biggest open risks to test first: (a) iroh-blobs/bao memory and resume behaviour on a real 50 GB+ video file on Windows; (b) relay egress cost when hole punching fails (CGNAT on SA mobile networks); (c) Loro doc size and load time at 100k assets with sharding; (d) whether iroh's PQ claim and the FAQ's "not post-quantum" are reconciled.
