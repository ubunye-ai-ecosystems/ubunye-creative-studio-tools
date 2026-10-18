# Kopano research (2026-10-17)

Kopano is the application-neutral collaboration protocol for Ubunye Creative Studio: Git-style versions, history and sync for photo, image and video projects, with media kept on people's own devices and identity from ThabangVision accounts.

This folder is research and a prototype, not product code. Branch: `research/kopano-architecture`.

| File | What it is |
|---|---|
| [01-baseline.md](01-baseline.md) | A. What exists today, what is reusable, gaps, risks, and what plan 08 got wrong |
| [02-architecture.md](02-architecture.md) | B. Architecture decisions, diagram, rejected alternatives, threat summary |
| [03-performance.md](03-performance.md) | C. Benchmarks E1 to E10 with environment, bottlenecks, projections |
| [04-proof-of-concept.md](04-proof-of-concept.md) | D. Two-peer proof of concept over iroh |
| [05-roadmap.md](05-roadmap.md) | E. Production phases with tests and exit criteria |
| [06-security.md](06-security.md) | G. Threat model, attacks tried, fixes, and what is still open |
| [tech-assessment-raw.md](tech-assessment-raw.md) | Technology research with sources and a decision matrix |
| [`../../08-collaboration-plan.md`](../../08-collaboration-plan.md) | F. The original plan, with a dated revision at the top |
| `lab/` | Rust prototype: `kopano-core` (neutral core), `kopano-lab` (toy editor, Light connector, benchmarks, two-peer `peer` binary) |
| `results/` | Raw outputs of every experiment |

## Status at a glance

| | Implemented | Tested | Measured | Proposed | Unverified |
|---|---|---|---|---|---|
| Content-addressed object store (BLAKE3), redb backend, one transaction per save | ✅ | ✅ | ✅ E7 | | |
| Canonical hash-trie manifest (1M items, 23 KB per edit) | ✅ | ✅ | ✅ E4 | | |
| Device keys + identity-provider certificates (offline check) | ✅ | ✅ | | ThabangVision link flow (098) | Supabase prod JWT algorithm |
| Signed versions, permission recomputed from data | ✅ | ✅ | | membership log, revocation list | |
| Per-setting three-way merge; clashes kept as records | ✅ | ✅ | | conflict review UI | |
| Connector contract v1 + 7-check conformance kit | ✅ | ✅ toy editor + **real Light engine** | | change feed (contract v1.1) | |
| Two-peer sync over iroh 1.3 (direct and relay) | ✅ | ✅ | ✅ E5, E10 | own relay, presence hints | hole-punch rate on SA mobile networks |
| Verified, resumable media transfer (iroh-blobs 0.103) | ✅ | ✅ | ✅ E5, E9 | pin+audit or own protocol on bao-tree | production fitness of iroh-blobs |
| Chunking policy (whole file for originals, FastCDC 64 KiB for mutable docs) | benchmark only | | ✅ E2 | in the media layer | |
| RAW pack for every camera (28/28 byte-identical, 13 brands; 42 to 99%) + progressive original: layer one = the app's own preview (Light smart preview, 0.6 to 1.8 MB) | lab Python + Rust `lj92` | ✅ 28 files; ✅ Light engine; ✅ Cape Town Linux peer (4/4 identical, renders within 1/255) | ✅ E11, E12, E13 | `preview` capability in the connector contract; per-format precomps (Nikon first) | desktop app window on the receiver, phones, own relay, direct-vs-relay path |
| Studio PC as always-on peer | | | | ✅ | |
| Film used-segment range fetch | | | | ✅ | byte-range mapping |
| Real two-machine, two-network test | | | | ✅ phase 3 | everything network-distance related beyond one relay |

## Recommendation in one paragraph
Build Kopano as a small Rust service per device: a redb object store, a signed version DAG over a canonical hash-trie manifest, per-setting merges with recorded clashes, device certificates issued by ThabangVision, iroh for every connection, and media moved only on request with verified resumable transfer. Apps connect through a five-call contract over their existing control ports, so their upstream code stays untouched. Next milestone: phase 1, the Kopano service plus the Light connector for one person on several devices, with a change feed so saves stay fast at 10k+ photos.

## Reproduce
```
cd research/kopano/lab
set CARGO_TARGET_DIR outside the repo
cargo test -p kopano-core -p kopano-lab              # core + toy-editor conformance
KOPANO_LIGHT_CLI=".../lightcraft-cli.exe" cargo test --release -p kopano-lab --lib ubunye_light -- --nocapture
cargo build --release -p kopano-lab                  # chunk_bench, hash_bench, manifest_bench, store_bench, peer
bash scenarios/poc.sh <peer.exe> <scratch> <photo1> <photo2> <photo3> <video>
python scenarios/fanout.py <peer.exe> <scratch> 2 5 10 25
```
