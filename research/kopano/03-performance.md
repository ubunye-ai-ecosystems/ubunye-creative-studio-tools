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
