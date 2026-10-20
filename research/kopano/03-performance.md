# C. Performance engineering report

## Test environment (all measurements)
- One PC: AMD Ryzen 7 3700X (8 cores / 16 threads), 16 GB RAM, Windows 11 Pro 22621.
- Disks: 256 GB SATA SSD (C:), 1 TB Seagate ST1000DX002 hybrid hard drive (T:). Unless stated, data lived on T: (hard drive).
- Rust 1.97.0, release builds with debug info. iroh 1.3.0, iroh-blobs 0.103.1, redb 4.4.0, fastcdc 3.2.1, blake3 1.x.
- Real media from the owner's local footage (Sony ARW 49 MB, camera JPEG 4 MB, XAVC S MP4 25 to 553 MB). Synthetic data only where stated.
- **No second machine, no WAN between two ends, no packet-loss injection.** The relay test is real internet (Johannesburg to n0's Frankfurt relay and back to the same PC).

Raw outputs are in `results/`. Every number below is copied from those files; nothing is estimated unless the row says "projection".

## E1. Edit metadata vs media (the core hypothesis)
Hypothesis: "Syncing edit instructions instead of media makes a small edit cost bytes, not megabytes, when the original is already on both devices." **Confirmed.**

| Measured | Bytes on the wire | Time (LAN path, same PC) |
|---|---|---|
| Join a 4-item project (history + edit state, no media) | 2,211 | 47 to 72 ms |
| One exposure change | 1,438 | 7 ms |
| Same change, relay only | 1,431 | 1.2 s connect + 2.6 s transfer |
| The original it refers to (ARW) | 0 (already held) | — |
| Versus sending the original | 49,325,568 | — |

A one-setting change is about 34,000 times smaller than the RAW it edits.

## E2. Chunking strategies on real files (`results/e2-chunking.csv`)
Bytes the receiver must fetch, given it already holds the previous file:

| Change | Whole file | Fixed 64 KiB | Fixed 1 MiB | **FastCDC 64 KiB** | FastCDC 1 MiB |
|---|---|---|---|---|---|
| RAW unchanged | 0% | 0% | 0% | 0% | 0% |
| RAW 64 bytes changed in place | 100% | 0.13% | 2.13% | 0.14% | 2.88% |
| RAW 4 KB inserted (shift) | 100% | 100% | 100% | **0.14%** | 2.89% |
| JPEG EXIF block removed (shift) | 100% | 100% | 100% | **7.4%** | 30% |
| JPEG re-exported | 100% | 100% | 100% | 100% | 100% |
| 149 MB layered doc, 5% tiles changed | 100% | 100% | 100% | **7.7%** | 36% |
| MP4 remux (faststart, data track dropped) | 100% | 100% | 100% | **36%** | 99% |
| MP4 trim by stream copy | 100% | 100% | 100% | **37%** | 99% |
| MP4 grew (appended recording) | 100% | 37% | 39% | 38% | 40% |
| MP4 re-encoded | 100% | 100% | 100% | 100% | 100% |

Speed (in memory, single thread): fixed-size chunking + BLAKE3 3.3 to 3.8 GB/s; FastCDC 64 KiB + BLAKE3 1.6 to 1.8 GB/s. Index overhead at 64 KiB: about 40 bytes per chunk, 0.06% of the file.

**Conclusions.** Content-defined chunking at ~64 KiB is the only method that survives insertions, and 1 MiB chunks lose most of the benefit. Re-encoding defeats every method. Camera originals never change, so whole-file hashing is enough for them. Chunking cost is far below disk speed (see E3), so it is not a bottleneck.

## E3. Hashing originals (`results/e3-*.csv`)
| Source | Throughput |
|---|---|
| 553 MB MP4, cold, hard drive | 135 MB/s |
| 20 ARW files (981 MB), cold, hard drive | 79 MB/s |
| Same MP4, warm (page cache) | 1,405 MB/s |

Bounded memory: 1 MiB read buffer. Hashing is disk-bound by a factor of 10 or more. **Decision:** hash at import while the file is read anyway; never re-hash unless size or modified time changes.

## E4. Manifest at scale (`results/e4-manifest.csv`, in-memory store)
| Items | Design | New bytes for a 1-item edit | Edit time | Diff 1 change | New bytes for a 1% edit |
|---|---|---|---|---|---|
| 10,000 | hash trie | 13.7 KB | 0.09 ms | 0.11 ms | 294 KB |
| 10,000 | flat list | 552 KB | 1.0 ms | 0.07 ms | 700 KB (projection) |
| 100,000 | hash trie | 20.1 KB | 0.24 ms | 0.12 ms | 3.1 MB |
| 100,000 | flat list | 5.5 MB | 17.9 ms | 1.4 ms | 7.0 MB (projection) |
| 1,000,000 | hash trie | 22.9 KB | 0.18 ms | 0.12 ms | 20.4 MB |
| 1,000,000 | flat list | 55 MB | 132.7 ms | 9.2 ms | 70 MB (projection) |

A save's size grows with the change, not with the catalogue.

## E5. Two-peer proof of concept over iroh (`results/e5-poc-run2.log`)
See [04-proof-of-concept.md](04-proof-of-concept.md). Network numbers:

| Path | Connect | Small-edit sync | Media throughput |
|---|---|---|---|
| Direct (same PC via LAN address) | 4 to 13 ms | 1 to 10 ms (3 round trips) | 1.2 to 1.7 Gbit/s (553 MB) |
| Relay only (Frankfurt) | 1.2 s | 2.6 s (3 round trips, about 0.9 s each) | not measured with private media (never sent through a public relay) |

Relay latency dominates. **Improvement loop (E10, `results/e10-*.log`):** the first design used one round trip per manifest level plus one each for identity and heads (5 in all). Two changes, measured over the same Frankfurt relay, 1-item edit:

| Sync design | Round trips | Transfer time (excl. 1.2 s connect) |
|---|---|---|
| Level by level (original) | 5 | 2.6 to 3.3 s (4 comparable runs; one outlier 7.7 s) |
| One "send me everything I lack" response | 3 | 2.1 to 2.6 s (3 runs) |
| **Same, with identity, heads and the request pipelined in one flight** | **1** | **0.83 to 1.13 s (4 runs)** |

Caveat: in the last batch three "level by level" runs reused edit values Bob already held (1 object moved), so only one of them (2.99 s) is a like-for-like comparison. Separately, one relay connection took **50.7 s** to establish (others 1.2 s): relay connection time has a long tail, unexplained, to investigate in phase 3.

Endpoint start-up: 3.7 s until online with n0's default discovery. A long-running Kopano service pays this once per boot, not per sync.

Offline peer: dialling waits 30 s before failing. Needs presence hints (roadmap phase 3).

## E6. Catalogue sync at scale (`results/e6-catalogue-sync*.log`)
| Items | Store | Owner's first save | Teammate's first sync (transfer) | Objects / bytes | Small edit save | Incremental sync |
|---|---|---|---|---|---|---|
| 10,000 | file per object | 12.4 s (incl. setup) | 12.9 s | 10,258 / 3.3 MB | 285 ms | 9 ms, 4 objects, 13.7 KB |
| 10,000 | **redb** | 2.8 s (incl. setup) | **0.32 s** | 10,258 / 3.3 MB | 264 ms | 18 ms, 4 objects, 13.7 KB |
| 100,000 | file per object | 447 s | failed (connection timed out: blocking reads starved the connection) | 151,549 / 35 MB | 6.3 s | failed |
| 100,000 | **redb** | **15.0 s** (incl. setup) | **4.8 s** (+1.0 s checks; same in 1-round-trip mode) | 151,548 / 35 MB | 1.7 s | **11 ms**, 5 objects, 20 KB |

First attempt at 100k on redb timed out ("connection lost") while another process was deleting 150k files on the same hard drive; clean reruns of both sync modes succeeded (`results/e6-catalogue-sync-redb-100k.log`). Lesson: a long local disk stall can exceed QUIC's idle timeout; the service must keep connections alive and do disk work off the network threads.

The small-edit save at 10k (264 ms) is dominated by the connector's full snapshot of every item on every save, not by Kopano's data structures (E4: 0.09 ms). **Decision:** add a change feed to the connector contract (Light: its `catalog.log`; Photo/Film: poll `revision` then diff only what changed).

## E7. Object store (`results/e7-store-*.csv`, 20,000 objects of ~330 bytes)
| Store | Disk | Write | Read (with re-hash) | Disk use |
|---|---|---|---|---|
| file per object | SSD | 27.4 s (729/s) | 1.8 s | 6.5 MB data (NTFS clusters make it ~80 MB) |
| **redb, 5,000 per transaction** | SSD | **0.43 s (47,000/s)** | **0.026 s** | 33.7 MB (pre-allocated pages) |
| redb, 1 transaction per object | SSD | 1.2k/s | | |
| file per object | HDD | 22.5 s | 2.0 s | |
| **redb, batched** | HDD | **0.94 s** | **0.035 s** | |
| redb, 1 transaction per object | HDD | **14/s** | | |

Batching is mandatory on hard drives (each durable commit waits for the disk). Deleting the 150k loose files of the old 100k test took over 30 minutes on the hard drive: the loose-file layout also makes clean-up slow.

## E8. Team fan-out (`results/e8-fanout.log`)
One owner serves; N teammates (separate processes, separate keys) pull one small edit at the same moment. All on this one PC, so this measures protocol, CPU and disk contention, not distance.

| Teammates | All applied | Time until all had it (incl. starting N processes) | Transfer per pull: median / max |
|---|---|---|---|
| 2 | 2/2 | 1.36 s | 34 / 34 ms |
| 5 | 5/5 | 1.32 s | 46 / 66 ms |
| 10 | 10/10 | 1.39 s | 146 / 158 ms |
| 25 | 25/25 | 5.7 s | 136 / 563 ms |

One serving peer handles 25 simultaneous pulls; the 25-peer wall time is dominated by starting 25 programs at once. Gossip is not needed at this size (decision D-8 stands); re-measure with real machines in phase 4.

## E9. Memory while moving large media (`results/e9-media-memory.log`)
553 MB original, client and server peers: peak resident memory **28.8 MB** (client) and **27.8 MB** (server). Verification overhead on the wire: 0.39% (555,854,799 bytes read for 553,691,975 of payload). This run's throughput was 33 Mbit/s because another process was deleting files on the same hard drive; the uncontended runs (E5) reached 1.2 to 1.7 Gbit/s.

## E11. How far can compression go? (`results/e11-compression.json`, `lab/scenarios/e11_*.py`)
Same PC. Sony ARW (uncompressed RAW, 24 MP, 14-bit), the camera JPEG of the same shot, and a develop-settings message.

| What | Method | Size vs original | Time | Original bytes back exactly? |
|---|---|---|---|---|
| ARW file | zstd level 3 | 78% | 0.5 s | yes |
| ARW file | xz level 9 | 57% | 27 s | yes |
| ARW sensor data | JPEG XL lossless (4 Bayer planes) | 44% | 1.9 s | yes (pixels) |
| ARW sensor data | JPEG-LS (4 Bayer planes) | 44% | 0.7 s | yes (pixels) |
| **ARW "raw pack"**: JPEG-LS sensor + zstd for the rest of the file | **44.7 to 45.5% (2 files)** | **0.9 s pack, 0.8 s unpack** | **yes, byte-identical file** |
| Camera JPEG | zstd level 19 | 98% | 0.9 s | yes |
| ARW | proxy, JPEG q85, 2012×3012 | 1.1% | 0.05 s | no (preview for editing) |
| ARW | proxy, JPEG XL d1.0, 2012×3012 | 0.9% | 4.4 s | no |
| Develop settings (508 B) | zstd level 19 | 57% | 0 | yes |

The noise floor: a simple left-neighbour predictor leaves 7.5 bits of entropy per 14-bit sample. JPEG-LS reaches about 7.1. What is left is mostly sensor noise, which no algorithm can predict, so heavier models (including LLM-based compressors such as LMCompress, which reports about 2× over JPEG XL on 8-bit photos) can only take a few more percent off camera RAW, at far higher cost. Not measured here.

Not measured: JPEG XL's bit-exact JPEG repack (published at about 20% smaller); this imagecodecs build has no brunsli/JPEG-transcode codec. Other RAW formats (CR3, NEF lossless, Sony compressed RAW) are already compressed and will gain much less.

**Decisions.** (1) Move uncompressed RAW originals as a "raw pack" (transport encoding only; the original's BLAKE3 stays its identity and is checked after unpacking). (2) Proxies first, originals on request: a 0.5 MB proxy is enough to start editing. (3) Do not compress edit messages; at 1.4 KB, round trips matter and bytes do not.

## E12. RAW pack for every camera + progressive original (`results/e12-*`, `lab/scenarios/rawpack.py`, `lab/kopano-lab/src/bin/lj92.rs`)
28 RAW files: 24 public CC0 samples from raw.pixls.us (13 brands: Canon CR2/CR3, Nikon NEF incl. HE, Fujifilm, Panasonic, Olympus, Pentax PEF/DNG, Sony, Leica, Hasselblad, Phase One, Apple, Google, Samsung, DJI, GoPro) plus 4 of the owner's own (Sony A7 III, Canon 700D, Nikon D5300, a Lightroom DNG). Same PC as above.

**Rule: every camera works.** `pack` tries methods from best to plainest and keeps the smallest one whose unpack gives back the original file byte for byte; storing the file as it is always qualifies. Result: **28 of 28 byte-identical**, including the two formats the RAW decoder cannot read (GoPro GPR, Nikon HE).

| Method | What it does | Where it applied | Packed size |
|---|---|---|---|
| `sensor` | finds plainly stored sensor samples (16-bit LE/BE or bit-packed, any row stride), JPEG-LS per colour plane | uncompressed Sony, Nikon, Fujifilm, Leica, DJI | **42 to 59%** |
| `lj92` | precomp for lossless JPEG: decode every stream, re-encode with the file's own tables, accept only if the bytes match, then JPEG-LS on the samples | Canon CR2, Apple/Google/Samsung/Pentax/Lightroom DNG | **82 to 90%** (97% for the Lightroom DNG, mostly non-RAW data) |
| `generic` | embedded JPEG previews repacked by JPEG XL (byte-exact), the rest zstd 19 | everything else (CR3, Fuji compressed, Nikon lossless/HE, Panasonic, Olympus, Pentax PEF, Hasselblad, Phase One, Sony lossless/lossy, GoPro) | **87 to 99%** |

Total: **936 MB to 729 MB (77.9%)**. The `lj92` tool rebuilt every stream it found on the first attempt (0 rejected), including tiled DNGs (192 and 384 tiles), the iPhone's predictor 7 and Samsung's restart markers. Camera JPEGs repack to 78% byte-exactly (closes the E11 open item).

**Progressive original.** Layer one is a 2560 px preview of linear camera data (no white balance, no clipping, no tone curve), gamma-encoded and stored as 16-bit JPEG XL at distance 0.7: **83 KB to 1.3 MB, median about 330 KB**. Layer two is the pack. Edit test: the same edit (exposure −2, +2, warmer white balance) applied to the preview and to the full RAW at the same size, counting 8×8 areas whose average colour differs by more than 2 levels of 255:

| | Areas off, exposure +2 (worst case) | Areas off, warmer WB |
|---|---|---|
| RAW-like preview (ours) | median about 0.5%, worst 4.9% (Samsung), 2.9% (Panasonic) | under 0.2% on the owner's files |
| 8-bit JPEG preview (the usual shortcut), q90 | 4 to 19% | up to 3.3% |

So edits made on layer one match the original far better than edits on a normal JPEG proxy, at a smaller size. Times to receive each layer are in `results/e12-summary.md` (arithmetic from measured bytes, not a network run): at 10 Mbit/s layer one takes 0.1 to 1 s and layer two 4 s to 1 min.

Gaps, stated plainly: (1) GoPro GPR has no layer one (no embedded JPEG; needs GoPro's open-source VC-5 decoder). (2) Nikon HE and GPR get no RAW-like preview; Nikon HE gets a view-only 1.6 MB resized camera JPEG. (3) Formats with their own lossless codecs (Nikon lossless NEF, Sony lossless, Fujifilm, Panasonic, Olympus, Pentax PEF, Hasselblad, CR3) only gain from the JPEG-preview layer; each needs its own precomp like `lj92`, and the owner's Nikon D5300 is the first candidate (pixel headroom about 10%). (4) Lossy RAW (Sony lossy, Canon C-RAW) cannot be repacked much by any exact method. (5) The pack is lab Python + Rust, slow (about 1 to 2 min per file, mostly the zstd 19 fallback); production would use a faster level and run in the Kopano service. (6) The editor stand-in is a neutral render (WB, exposure, gamma), not the Light engine.

## E13. Real use: the Light engine, a second machine in Cape Town, Linux (`results/e13-*`, `lab/scenarios/e13_*`)
The owner asked whether E12 was tested in the real app and in another place. It was not: E12 used a stand-in renderer on one PC. E13 does it properly.

**1. Looked at the images.** Rendered the same edit on the RAW, the E12 preview and a JPEG proxy and viewed them. The E12 preview recovered highlights like the RAW; the JPEG did not. But the stand-in renderer itself was crude (no colour profile, no rotation).

**2. Light engine (`lightcraft-cli`), this PC.** The E12 preview, written as a linear DNG, opened in Light but rendered about one stop darker and warmer than the RAW at every exposure (95 to 100% of areas off). Cause: Light develops each camera with its own baseline exposure, colour fit and tone curve, which a generic file does not carry. **Correction: layer one must come from the app's own engine.** Light already has this: Smart Previews (`library.smartPreviews`, `crates/engine/src/smart.rs`), about 0.6 to 1.8 MB, editable while the original is offline. Measured on 5 cameras (3 of the owner's), original hidden, same edit:

| | Exposure −2 | No edit | Exposure +2 |
|---|---|---|---|
| Areas off by more than 2/255 | 0 to 0.2% | 0 to 1.1% | 0 to 13% (Canon 700D) |
| Mean difference (levels of 255) | 0.04 to 0.22 | 0.07 to 0.42 | 0.12 to 0.69 |

Viewed side by side: no visible difference at screen size; at 100% the smart preview is slightly softer (it is smaller than the 2560 px export and is upscaled).

**3. Second machine, another city, another OS.** A Vercel Sandbox in Cape Town (cpt1; Ubuntu 26.04, 4 vCPU), driven with the Vercel CLI. Only CC0 samples left this PC (Canon 5D IV CR2, Sony A7 IV ARW, Nikon Z6 lossless NEF, Fujifilm X-T4 RAF). Transfer: `sendme` 0.36.0 (n0's iroh + iroh-blobs tool, same stack as Kopano's peer), Johannesburg home connection to Cape Town.

| Step | Result |
|---|---|
| Light built from source on Linux (`develop`, b94bf8b) | 234 s; zig as the C linker (the image's apt could not verify signatures; not bypassed) |
| Layer one: Light library + 4 smart previews, 3.4 MB | received in **11.2 s** (incl. start-up and finding the peer) |
| Light on Linux, originals absent | developed all 4 photos at −2/0/+2 from the smart previews (`originalOnline=false`), 6 s |
| Layer two: 4 packs, 104 MB (from 126 MB of originals) | received in **308 s (about 2.7 Mbit/s)** |
| Unpack on Linux, SHA-256 against this PC's manifest | **4 of 4 byte-identical** (lj92, sensor, generic), 0.3 to 1.6 s each |
| Light on Linux vs Light on Windows, same files and edits | **max pixel difference 1/255** in all 24 renders (rounding; builds differ: b94bf8b vs dfc548bc) |
| Renders sent back Cape Town → this PC, 155 MB | 425 s (about 2.9 Mbit/s) |

The speed limit both ways was this PC's internet line, not the protocol. Whether iroh went direct or through a relay was not recorded. Two early failures were mine (a crashed background `sendme`, which needs a terminal, and a mis-copied ticket), not the network.

**What E13 changes.** (1) Kopano core stays app-neutral; the connector contract gains a `preview` capability: the app makes layer one with its own engine (Light: smart preview). Kopano moves it and the pack. (2) The RAW pack stays Kopano's (app-neutral, byte-exact, proven across OS). (3) Light smart previews are 8-bit JPEG inside; a 16-bit version could tighten the +2 EV case (a Light change, proposed only). Not tested yet: the desktop app's own window on the receiving side (only the headless engine), phones, a second real PC, a relay we run ourselves.

## E14. Collaboration scenarios, our own relay, the full Light UI, phone size (`results/e14-*`, `kopano-lab/tests/collab_scenarios.rs`, `lab/scenarios/e14_relay_run.sh`)

**Collaboration suite (25 scenarios, in process, every new version verified parents-first as the network pull does).** Test app with three lanes per photo (develop, selects, notes), like the Light connector. `cargo test -p kopano-lab --test collab_scenarios -- --nocapture --test-threads 1`, 27 s. **22 behave correctly, 3 are gaps.**

| Group | Scenarios that behave correctly |
|---|---|
| Basic flow | A1 join and receive (50 photos, no media); A2 an edit reaches a teammate; A3 work passes through a teammate while its author is offline, still verified as the author's |
| Working at the same time | B1 different photos; B2 same photo, different settings (both kept); B3 same setting (clash record keeps both, travels to everyone); B5 three people offline merged in two orders end identical; B6 delete vs edit (photo stays deleted, the edit is kept in a clash record, never silently lost); B7 two cards imported at once; B8 a week offline, 300 versions each on 2,000 photos, merged in 1.05 s; B9 ten people, 200 random edits, random pairwise syncs: everyone identical at the end |
| Roles and permissions | C1 viewer refused; C2 client may comment, not edit; C3 retoucher limited to develop; C4 non-member refused; C5 a forbidden change hidden under a valid teammate's version refused; C6 a merge cannot smuggle a change; C7 one flipped bit rejected; C8 expired certificate refused |
| History and recovery | D1 restore keeps history and teammates follow; D2 restore while a teammate edits: both survive; D3 a wiped laptop gets everything back from a teammate |

| Gap | What we saw | What is needed |
|---|---|---|
| B4 resolving a clash | the clash record stays forever after someone picks a value (B9: 118 open records after 200 edits by 10 people) | a resolve step: a version that removes the record, allowed to whoever may write that lane |
| C9 remove a member / stolen laptop | accepted until the certificate expires | signed membership log + device revocation list checked on every sync |
| C10 add a member later | members are fixed at creation | the same membership log |

Also seen: B8 saving took 22.5 s for 600 saves on 2,000 photos (37 ms each, the full-snapshot cost from E6); the change feed fixes it.

**Our own relay.** `iroh-relay` v1.3.0 (n0's published binary, `--dev` mode) in a Vercel Sandbox in Cape Town behind its HTTPS route; the peer takes `KOPANO_RELAY_URL`. Alice on this PC (Johannesburg home line), Bob's peer built and run in the same sandbox. One small edit (4 objects, about 11.6 KB), 3 runs each, path as reported by iroh:

| Route | Connect | Transfer |
|---|---|---|
| **Our relay, Cape Town** (relay only) | 83 to 150 ms | 237 to 247 ms |
| n0 public relay, Frankfurt (relay only) | 837 to 1,635 ms | 879 to 1,674 ms |
| Our relay, direct allowed | 81 to 153 ms | 182 to 247 ms (still relayed: the sandbox allowed no direct path) |
| First sync of a 1,000-photo project, our relay | 118 ms | 1,127 ms (1,249 objects, 341 KB) |

About 6 to 8 times faster than the European relay. Caveat: Bob sat next to the relay; two South African homes would both cross local networks. A sandbox is not a production host (45-minute limit); production needs a small always-on server in South Africa (proposed, not priced).

**Full Light UI.** `lightcraft-cli snapshot` (the whole desktop app drawn headlessly, driven only through Light's control channel, as Light's own rules require) on the E13 layer-one library with the originals absent: the Develop view showed the photo from its smart preview, Exposure −1.50 applied, the filmstrip marked each photo as a smart preview, `originalOnline=false`. **Phone size (390 × 844):** the desktop layout breaks (photo shrinks to a thumbnail, top-bar icons overlap the search box). Light needs a phone layout; there is no phone build of Light or Kopano to test on a real phone.

**Second PC at home.** Not possible to simulate here: virtualisation is off in this PC's firmware (no Windows Sandbox or Hyper-V). A cloud Windows machine would not share the home router. Needs a real second device.

## E15 and E16. Real devices: home laptop (same network) and Soshanguve laptop (another network, about 45 km away)
A test helper (`agent`, Windows and macOS builds) on each laptop ran a fixed list of Kopano steps on instruction from this PC's key only. Findings, in order of importance:
- **Two and three collaborators converged to identical state** (fingerprint over all 1,005 items equal on all three devices), including true merges, a three-way clash, laptop-to-laptop work while the owner was offline, laptop-to-laptop media, relay-only, and an outsider ("Carol") refused by the Soshanguve laptop.
- **Each device rendered the same final image** from its own synced settings (one photo, three people's edits): byte-identical JPEG on all three (`4fe1361d505410cf`).
- **One identity must never run in two programs at once** (the control tool shared this PC's identity; the relay delivered sync requests to the wrong program). Production: a controller needs its own key.
- Windows Smart App Control blocks unsigned programs with no override: **code signing is a product requirement**. Two support issues seen: running from inside the zip, and the firewall prompt.
- n0's nearest public relays were Frankfurt, US East or Asia; our own Cape Town relay gave 0.4 to 0.7 s connects and 0.3 s small syncs to Soshanguve.
- Find my device (owner's own and his brother's laptops, with consent): Windows location service gave 55 and 63 m accuracy; the IP address guess was 27 to 40 km off; the open BeaconDB had no data for these areas.

## E17. Reliability simulator (`kopano-lab/tests/sim_fuzz.rs`, `sim_workflow.rs`)
Random teams (2 to 6 people, mixed roles, some cheating), random edits, imports, deletes, restores, member changes, syncs (15% cut off part way). Invariants: agreement, no silent loss, safety, no stuck peer.
- **Found and fixed: a sync cut off at the wrong moment left the receiver stuck forever** (53 of 100 runs). Cause: the copier stored a version before its parents. Fix: store a version only after its whole closure (`copy_version_closure`). After: 0 stuck.
- **Found: one shared line merged by everyone does not always converge** (38 of 200 runs; "mine wins" on clashes makes concurrent merges disagree; this is also why 10 people produced 118 clash records). Partial fix: clash outcome identical on every device. Real fix: the owner's model below.
- **v2 workflow (owner's model): main owned by the creator, one branch per person, approvals into main, signed members log** (`kopano-core/src/workflow.rs`). 500 runs x 300 actions: 34,197 edits, 5,414 approvals, 2,981 member changes, 7,717 cheating attempts, 4,616 cut transfers. First run: 447/500 passed. Fixes: catch-up used the genesis permission check (`integrate_checked`); work judged by the members its update cites (not by what the receiver holds on arrival); a mechanical merge is checkable so anyone may make it; every device judges its own work by the same rules. **Final: 500/500 passed** (38,236 edits, 6,266 approvals, 3,401 member changes, 7,252 cheating attempts, 5,193 cut transfers). No silent loss, no safety violation, no stuck peer.
- **Found and fixed (2026-10-20): a viewer's merge that recorded a clash was refused by everyone, so that viewer never converged** (shared-line sim: 9 of 200 seeds "did not converge after 7 rounds"; the same heads for 30 rounds, so a real divergence, not slow convergence). A viewer may not write the clash lane, and the permission check only let through settings merged one by one, not the whole-item pick or the clash record. Fix (`merge::merge_one`, `project::reproduces_merge`): a merge value or clash record that the receiver can recompute exactly from the two verified parents, against a best common ancestor and either merge policy, is mechanical and allowed to anyone. A made-up value or record inside a merge is still refused (`repo::criss_cross::a_viewers_merge_that_records_a_clash_is_accepted`). After: 1,000 of 1,000 seeds converge, all in the first final round; `sim_fuzz` now fails on any divergence, stuck peer or silent loss. Its loss check also accepts an A-B-A edit (put an item back exactly as it was) when the person who then changed the item saw that very item (2 of 1,000 seeds; three-way merging cannot tell it from no change).

## E18 to E20. Bad networks, compact sync, open connections (`chaos.rs`, `scenarios/e18_net.sh`, `tests/e19_bytes.rs`)
Local iroh-relay behind a proxy that adds one-way delay, caps bandwidth and drops connections; both peers relay-only (pessimistic: every message crosses the bad link twice).

| Network | Join 1,000 photos: before / compact | One edit: before / compact / on an open connection | 4 MB photo |
|---|---|---|---|
| Local | 0.61 s | 0.53 s / 0.53 s / - | 0.8 s |
| Fibre 50 Mbit/s, 10 ms | 1.1 s | 0.76 s | 2.1 s |
| LTE 10 Mbit/s, 30 ms | 2.1 s | 1.1 s / - / **0.17 s** | 5.4 s |
| 3G 1 Mbit/s, 150 ms | 7.7 s / **5.3 s** | 3.9 s / 3.0 s / **0.62 s** | 40 s |
| EDGE 100 kbit/s, 300 ms | 37.7 s / **15.8 s** | 10.0 s / 6.8 s / **1.28 s** | 358 s (92% of the link) |
| Flaky (3G, all connections dropped every 20 s) | 7.7 s / 5.4 s | 3.6 s / 3.1 s | **44 s, one attempt, no restart** |

- **Bytes of one edit: 11.7 KB to 0.97 KB** (1,000 photos), and constant with project size: 13.3 KB to 0.9 KB at 10,000, 20.1 KB to 1.0 KB at 100,000. Method: send each changed tree node as its difference from the node the receiver already holds (rebuilt exactly and hash-checked), deflate the rest. Deflate alone barely helps an edit (hashes do not compress).
- **Join: 342 KB to 92 KB** (deflate over the whole reply; 3.3 to 3.8 x at 1k to 100k photos).
- **On an open connection a sync is exactly one round trip** (EDGE: 4 x 300 ms = 1.2 s measured 1.28 s). Production: a long-running Kopano service keeps connections open; an edit on EDGE goes from 10 s to about 1.4 s.

**E20 always-on daemon with push** (`peer daemon`, `scenarios/e20_push.py`): one process per device holds the project, serves, takes app edits on a local port and pushes each new version with the change itself (compact, relative to what each watcher has). Edit on one device to applied on the teammate, median of 10, all 10/10 delivered on every profile:

| Network | Before tonight | Daemon, announce then pull | **Daemon, push the change** |
|---|---|---|---|
| LTE | 1.1 s | 0.30 s | **0.16 s** |
| 3G | 3.9 s | 1.0 s | **0.41 s** |
| EDGE | 10.0 s | 2.1 s | **1.02 s** (floor here: 2 x 300 ms + 2 x 970 B at 100 kbit/s) |
| Flaky | 3.6 s | 1.1 s | **0.59 s** |

**E21 media beyond photos.** Camera clip 553.7 MB (81 s, XAVC): 720p editing proxy 21.9 MB (4%, 25x smaller, 38 s to make). Its audio as WAV 15.5 MB: lossless pack (FLAC for the samples, header kept) **52%, rebuilt byte for byte**. Camera H.264 itself cannot be shrunk losslessly in a useful way: proxies first, originals on request.

**E22 a 4.3 GB original.** Import used 12 MB of memory. Download killed part way (process gone), restarted later: resumed at 2.46 GB, fetched only the remaining 1.84 GB, complete and verified. Speed was bound by the source hard drive (about 25 Mbit/s).

**E23 save cost at scale** (`tests/e23_scale.rs`, `Repo::save_changes`): the connector hands Kopano only what changed (change feed) instead of a full snapshot.

| Photos | Import | Save, full snapshot | **Save, change feed** |
|---|---|---|---|
| 10,000 | 0.1 s | 55 ms | **0.19 ms** |
| 100,000 | 1.0 s | 755 ms | **0.24 ms** |
| 300,000 | 4.5 s | 3.3 s | **0.6 ms** |
| 1,000,000 | 18.1 s | 13.8 s | **1.45 ms** |

**E24 one owner, many teammates** (`scenarios/e24_fanout_push.py`, all daemons on one machine, direct links): the owner's daemon pushes each edit to every follower.

| Teammates | All joined | Edits that reached all | Median | Slowest | Follower's own apply (median) |
|---|---|---|---|---|---|
| 10 | 5.7 s | 5 of 5 | 0.16 to 0.48 s | 0.63 s | 0.21 s |
| 25 | 7.3 s | 5 of 5 | 0.57 to 1.39 s | 1.71 s | 0.70 s |
| 50 | 8.9 s | 5 of 5 | 1.4 to 3.2 s | 3.5 s | 2.5 s |

Every edit reached every teammate (each push about 1 KB). The owner now builds each push once and shares it with all followers (first run, without that: 25 teammates slowest 2.5 s). What is left is the followers' own apply step, slow here only because 50 daemons share one CPU and one hard drive; on separate devices each applies in parallel. Not measured: 50 real devices.

## Bottleneck summary (what to optimise, in order)
1. **Round trips over relays** (E5): dominant cost of a small edit at a distance. Fixed in the lab (E10): one pipelined round trip, 3× faster over the Frankfurt relay.
2. **Connector full scans** (E6): O(catalogue) per save (264 ms at 10k, 1.7 s at 100k, while the core itself needs under 1 ms). Fix: change feed in the contract (phase 1).
3. **Small-file storage on Windows** (E7): fixed by redb; keep it.
4. Hashing and chunking are **not** bottlenecks (E2, E3): 10× faster than the disks.
5. Memory is bounded for media (E9).

## Cost projections (not measured)
- Edit traffic: about 1.5 KB per small edit (measured) × edits per day. Even 10,000 edits a day per project is about 15 MB of relay traffic: negligible.
- Media traffic dominates and only happens when someone fetches originals or proxies. With direct connections it costs nothing. Through our own relay it costs that relay's bandwidth; n0's paid plan quotes 100 GB/month for USD 19 (tech assessment, unverified for our use). Hole-punch success on SA mobile networks is unknown and decides this cost; measure in phase 3.
- Storage: redb overhead about 5× the raw object bytes for small objects (E7); 100k photos of edit state ~35 MB raw, so ~175 MB worst case on disk (projection). Compaction can reduce it.
