# H. Test programme: works for everyone, anywhere (started 2026-10-18)

Goal: when Kopano is published, it works. Not only on good Wi-Fi, not only for photos. Each experiment has a pass rule written **before** it runs. A failure is fixed and the experiment rerun. Results go in `results/`, conclusions here.

## The matrix

| Axis | Values tested |
|---|---|
| File size | 1 KB edits, 10,000 small files, 50 MB RAW, 550 MB camera clip, 4 GB+, synthetic 20 GB+ |
| Network | LAN, LTE, 3G, EDGE, flaky, high packet loss, relay only, **no internet at all** (LAN only), **no network at all** (USB / SD card), people never online at the same time |
| Failure | process killed at any instant, disk full, laptop asleep mid-transfer, corrupted disk, hostile peer |
| Machines | this desktop, home laptop, Soshanguve laptop (Windows), Linux cloud sandboxes, macOS build |

## Experiments

| # | Question | Pass rule | Status |
|---|---|---|---|
| E27 | Kill the process at a random instant during save, sync and media download, hundreds of times. Does anything break? | 0 damaged stores, 0 lost saved edits, every interrupted job finishes on restart | **PASS**: 400 rounds, 322 kills (131 saves, 86 syncs, 105 downloads of a 300 MB file): 0 damaged, 0 of 60 completed edits lost, 0 restart failures. Fix made first: atomic writes now flush to disk before the rename (power cut safety) |
| E28 | The disk fills up during a sync or download | Clear error, no damage, finishes once space is freed | **PASS** after 2 fixes, on a 400 MB virtual disk with 60 MB free: download refused up front with "needs 174 MB more" (was: "Receiver closed" after the store had reserved 151 MB it then threw away); 20,000-item sync with 2 MB free fails with "not enough space on the disk"; store healthy after both; after freeing space the sync and the 157 MB download complete |
| E29 | The laptop sleeps for minutes in the middle of a sync | Picks up by itself after waking, no restart from zero | **PASS after a fix.** Before: after a 2 min sleep the follower lost the connection ("timed out") and never received another edit. Fix: the daemon reconnects by itself (1, 2, 4 ... 30 s), saying what it already has, so everything missed comes as one push. After: follower slept 2 min, caught up 1.0 s after waking; owner slept 2 min, its edit arrived 3.1 s after waking. Interrupted downloads resume (E22, E27) |
| E30 | **Video, invented approach: watch and edit a remote original without downloading it.** Fetch only the byte ranges the timeline needs, each range verified | Bytes moved proportional to the seconds used, not the file size; segment plays; every byte verified | **PASS** on a 495 MB, 67 s Sony XAVC original: a 5 s cut moved 39.9 MB (8%), three 4 s cuts 95.3 MB, 2 more seconds 16.7 MB; every decoded frame identical to the full original (562 of 562, framemd5). Bug found on the way: the sparse copy needs the 16-byte header of the data box |
| E31 | One big file from several teammates at once (swarm) | Faster than from one; verified; survives a source leaving | **PASS** (new `fetch-swarm`: 4 MB stripes from a shared queue, endgame duplicates the last stripes; one network endpoint for all teammates, after a bug where two endpoints with one key made the relay drop one). 100 MB, each teammate on its own 16 Mbit/s link: one teammate 63.0 s; two **34.3 s** (1.84x); one leaving after 8 s: still complete (58.6 s, the other took over); fast 16 + slow 4 Mbit/s: 51.2 s, split 83/20 MB by speed (without endgame 56.4 s). Rig lesson: a relay knows each device once, so each teammate needs its own relay in a test |
| E32 | No internet: same Wi-Fi only; no network: USB stick | Sync works with the internet cable pulled; a USB bundle carries a day's work and is verified on arrival | **USB part PASS** (new `export-bundle` / `import-bundle`): 50 edits + 2 originals (124 MB) in one 130 MB file, imported and verified in 0.7 s; the next day's 5 edits: a 2.8 KB bundle; a damaged bundle and another project's bundle change nothing (plain error); a damaged original inside a bundle is rejected. Wi-Fi-only part: waits for E37 |
| E33 | Two people who are never online at the same time; a third carries the work | Converges through the carrier | **PASS**. **E33b sealed "data taxi"** (new `seal-bundle`, `kopano-core/src/seal.rs`): Alice's day (30 edits + 24 MB original) sealed for 2 member devices; Carol (not a member) carries it: cannot open ("not one of the bundle's recipients"), no original bytes or names visible, her one-byte change refused; Bob imports it intact. Before that: **PASS**: Alice and Bob never online together; Mpho synced with Alice, Alice went offline, Bob synced from Mpho: identical head, 7 edits applied |
| E34 | 10,000 small files (sound effects, stills) | Per-file cost small; no per-file round trips | **PASS** with collections (new `add-folder` / `fetch-folder`): 2,000 files, 42.5 MB. Direct: one request 11.3 s vs one per file 22.0 s. Far link (4 Mbit/s, 150 ms): **94.7 s vs 1,621.7 s, 17x faster**; one request uses 90% of the link |
| E35 | Direct connection with real packet loss (Linux netem) | Usable at 10% loss | |
| E36 | Long run: daemons editing for hours on a bad network | 0 divergence, flat memory | |
| E38 | Sekhoto past papers on a phone with almost no data: page by page, verified, phone to phone | Bytes per page opened, not per paper; works offline between two devices | **PASS after a fix.** Real Grade 11 Maths P1 (2015, 16 pages, 397 KB), split by the publisher into one verified object per page (page list: 1 KB; pages 1 to 191 KB, median 30 KB). No internet (LAN-only): learner A opened 3 pages for **116 KB**; with the publisher gone, learner B got page 3 **from learner A** (81 KB), byte-identical; a page A lacks fails cleanly. **Bug found:** `fetch-media` and `fetch-ranges` exited without closing the media store, so a just-downloaded small file could be lost; fixed |
| E37 | The three real machines: big file, swarm, LAN-only, live edits | Same as above on real hardware | |

## Proposed next (not built): Kopano Link

Kopano today is two things in one: a secure device-to-device connection and shared verified projects. Split them:

- **Kopano Link**: any device (phone, laptop, TV, a camera through a bridge, a $3 microcontroller) has a key, finds neighbours over any link, proves who it is, and announces **capability classes** defined by what hardware physically does, like USB device classes or PCIe configuration space: *sense* (camera sensor, microphone, GPS), *compute* (CPU, GPU, NPU), *show* (screen, speaker), *keep* (storage), *reach* (radios it can bridge). Permitted teammates call them: "render this on your GPU", "capture with your sensor". Requests signed, results fingerprinted.
- **MCP over Kopano Link**: MCP is JSON messages that need a pipe; Link is a pipe between proven devices, so an AI on one device can use tools on another with no cloud. Offline: instructions in sealed bundles ("do this when you get it").
- **Kopano Projects**: everything tested above.

The hardware asks are minimal and vendor-free: a processor that can hash and sign, some memory, any way to move bits.
