# 09. Kopano in the apps, signed in with ThabangVision

Owner direction (2026-10-19): Kopano is the co-editing protocol. People sign in with their
ThabangVision account (like GitHub), and every downloaded app (Light, Photo, Film; Windows, Mac,
Linux) is connected through it. Storage of finals and a central download point are parked.

This document fixes the architecture. It builds on 02 (protocol), 05 (roadmap phases 1 to 3) and
06 (security), and on what the real-machine tests (E37 to E39) taught.

## 1. Pieces

```
 ThabangVision (web, Supabase)         one per account
   accounts, "link this device", projects + invites (mirror), presence hints,
   Kopano issuer key (signs device certificates), revocation list
          ^  HTTPS, only for sign-in, renewal, invites, presence
          |
 kopanod (one background service per computer)
   device key (OS key store), certificates, project stores (redb + media),
   one iroh endpoint, live sync (push), local API on 127.0.0.1 with a token
          ^  local API (HTTP + server-sent events)
          |
 Light / Photo / Film (each app)
   "Kopano" panel: sign in, projects, teammates, live changes, clashes
   connector: turns app edits into Kopano items and back
```

## 2. Decisions and why

| Decision | Why (evidence) |
|---|---|
| ONE service per computer, apps talk to it; Kopano is not linked into each app | One device identity must never run in two programs: two helpers with one id answered each other's requests (E37c rehearsal) and a second endpoint with the same key was refused by the relay (E39). Three apps would mean three copies of the key and three endpoints. |
| Sign-in = OAuth 2.0 device flow (RFC 8628), like `gh auth login` | The app never sees a password or a Supabase token (06: "the desktop never receives a Supabase token"). Works on Windows, Mac and Linux the same way; the browser does the sign-in, including 2-step. |
| ThabangVision signs device certificates with a dedicated Kopano issuer key | Peers check certificates offline (no server call per sync). Not the Supabase JWT or service-role key (06). |
| Certificates last 30 days and renew silently | E37c: the 24 h certificates of the E15 team all expired and every machine was refused. Renewal = the device signs a challenge with its key; suspension or a lost device stops renewal and lands on the revocation list. |
| Membership truth stays in Kopano's signed members log; ThabangVision keeps a mirror for invites and "my projects" | Peers must decide offline who may edit (workflow.rs). The website is the place to find and invite people, not the authority peers depend on. |
| Presence hints via ThabangVision (endpoint id + relay URL, no IPs stored long) | Dialling an offline peer waits 30 s (E10); address files do not work for real users. |
| Small things over the relay, previews first, originals stay home, export where the originals are | Relays carry about 1 KB per edit; the 50 MB tests were worst cases (E37c/E39, owner 2026-10-19). |
| Local API on 127.0.0.1 with a per-user token file | Any program on the machine could otherwise drive the service. |
| Feature flag `kopano` off in ThabangVision until the owner approves | Production auth changes are owner review items (05 phase 2). |

## 3. Device link flow

1. The app (through kopanod) calls `POST /api/kopano/link/start` with the device public key.
   ThabangVision returns a short user code (`ABCD-1234`), a device code and a poll interval.
2. kopanod opens `https://thabangvision.com/link?code=ABCD-1234` in the browser.
3. The person signs in (normal ThabangVision sign-in) and approves "Link this computer: name, OS".
4. kopanod polls `POST /api/kopano/link/token` with the device code and a signature by the device
   key (proves it holds the key it registered). On approval it receives the device certificate
   (account id, username, device key, expiry 30 days, signed by the issuer key).
5. Renewal: `POST /api/kopano/cert/renew` with a fresh signature; refused if the device is revoked
   or the account suspended.
6. Peers trust the issuer key published at `/.well-known/kopano-keys.json` (pinned in the app,
   with rotation by a signed key list).

## 4. Data (ThabangVision, migration proposed in 05 as 098)

`kopano_devices` (id, user_id, device_key, name, os, created_at, last_seen, revoked_at),
`kopano_link_requests` (device_code hash, user_code, device_key, status, expires_at),
`kopano_projects` (id, owner_id, name, kopano_project_id, created_at),
`kopano_project_members` (project_id, user_id, role, invited_by, status),
`kopano_presence` (device_id, endpoint_id, relay_url, updated_at; rows older than 2 min ignored).
RLS: users read their own devices and projects they belong to; all writes through API routes with
server checks; no client inserts into device tables.

## 5. Build order (each slice ends tested)

| Slice | What | Done when |
|---|---|---|
| S1 | kopanod: background service, local API, live push between devices, lab certificates | Two services on this PC and one laptop: an edit appears on the others within 2 s with no pull command; survives sleep and network loss |
| S2 | ThabangVision link flow on DEV only (migration, API, /link page), kopanod `login` | A real dev account links this PC; peers accept its certificate; a revoked device is refused |
| S3 | Light panel: sign in, open/share project, teammates, live changes, clash choice; previews first | Two people edit the same shoot in Light and see each other's changes live |
| S4 | Windows, Mac, Linux builds and start-at-login for kopanod | Installed and running on all three from CI artifacts |
| S5 | Real test with the Soshanguve laptop | The "done when" of S3 on two houses |
| Later | Photo and Film connectors, finals sharing, own relay | |

Production adoption of S2 (ThabangVision) waits for the owner's review.

## 6. Slice "media": photos reach a teammate who does not have them

Before this slice both Light windows already had the same photos. Now a teammate whose Light
starts empty gets the owner's photos as previews, edits them with the team, and can ask for an
original when needed. Code: `lab/kopano-node/src/media.rs`; test: `lab/scenarios/s5_media.py`.

**How it works**

1. kopanod serves files over the same endpoint (iroh-blobs). Every connection and every request
   is checked: the device must have shown a current member's certificate on a project here, and
   the file must belong to a project that device is a member of. Knowing a hash is not enough.
2. Preview first. When the project has a photo the window does not show, kopanod asks an online
   teammate (owner's devices first) for a preview, fetches it, and puts it into Light. The
   preview is Light's own Smart Preview: the device with the original asks its window for it
   (`photo.exportSmartPreview {id, path}` on the control port; a .lcsp file, the scene-linear
   proxy plus the facts Light needs to render it as the original: content hash, name, kind,
   format, size, white balance, lens, camera fields). It keeps it, and makes them ahead of time
   for shared projects so a joiner does not wait. A device that only has the preview can hand
   it on. The asker says `accept: ["lcsp", "jpg"]`; the receiver tells the format from the
   file's first bytes.
3. The key. The member's kopanod adds the preview with `library.addFromSmartPreview
   {smartPreview, contentHash, fileName}`, and Light keys the photo by the ORIGINAL's content
   hash itself, so no alias is needed. Light names its file `smart-preview:/<hash>/<name>`
   (offline). The connector only learns which of these stand-ins are the project's (and the
   original's asset reference to publish); a stand-in it was not told about is never published,
   and once the original is in Light the original wins.
   Fallback for an older Light (no `photo.exportSmartPreview`, or the export fails): the device
   with the original renders a JPEG (`lightcraft-cli render`, 2560 px, sharpening and noise
   reduction off, as shot) and the member imports it under an alias by file path, as before.
4. Waiting photos are not deletions. A photo whose preview has not arrived still counts as the
   project's state (the core would otherwise read an empty window as "everything removed" and
   send that to the team). Changes for it wait in the project and are applied on arrival.
5. Original on request: API `fetch-original {project, key}`. The size is proven first and must fit
   on the disk, a cut download continues where it stopped (iroh-blobs keeps what arrived), and
   `original` events report MB done every second. The original is imported in place; Light
   hands the Smart Preview stand-in over to it under the same photo id, settings, history and
   stars kept (a JPEG preview is instead taken out after the original is given the project's
   settings).
6. Panel events, plain and small: `media {photos_waiting, photos_ready, photo, mb_done}`,
   `original {photo, state, mb_done, mb_total}`, `preview-made` (on the owner's side). API `media`
   says per photo whether it shows from its original, from a preview, or is still waiting.

**Real test (2026-10-20, this PC, the owner's 5 Sony ARWs, Light windows driven by its control
channel, kopanod with the S4 changes merged; screenshots in T:/kopano-lab/media/final)**

| Step | Result |
|---|---|
| Member1's Light starts empty, joins | 5 of 5 photos arrived as previews: 3.3 MB in total for 246.5 MB of originals (1.3 %); first photo 6.4 s, all 17.6 s after joining (renders 2-5 s each on Alice's side, made ahead) |
| Alice edits exposure on her RAW | Member1's preview follows in 0.9 s |
| Member1 edits contrast and stars on the preview | Alice's RAW follows in 1.3 s (stars 0.05 s): the key matches |
| Member1 fetches one original (49.4 MB) | 0.8 s transfer on this PC, 1.6 s to swap in Light; the RAW shows with exposure +1.25, contrast +40, 4 stars; still 5 photos |
| Alice edits again | Member1's original follows in 1.3 s; both heads equal |
| Carol (valid certificate, not a member) asks Alice for an original | refused at connect (Alice's log: `media: refused 6c1be93e`); her tool only sees "connection lost" |
| Regressions | S1 20/20 edits, median 123 ms, both ways, clashes settle, restart catch-up; S3 Light to Light 1.4 s exposure, 0.7 s stars, slider drag 1.2 s, same head |

**Limits found (honest)**

- The JPEG fallback is a picture to work on, not to judge tones on: same harness, edited 20.0/255.
- The member's Light keeps the Smart Preview in its own smart previews folder (a throwaway one
  in `--memory`); kopanod keeps its copy in `media/previews/<project>/`, so a restarted Light
  gets the photos back without the network.
- Light runs `photo.setMeta` only with a selection, even when `ids` is given. A stand-in is not
  selected the way an import is, so the connector now selects the photo first (as for develop).
  The first run without that took 45 s to show all photos (retries); a Light fix would be to
  check `ids` in that command's gate.
- A photo Light has no content hash for (key `name|captured`) cannot be shared as a Smart
  Preview; Light refuses the export and kopanod sends a JPEG.
- A kopanod from before this change asks without `accept` and gets a JPEG; one that sees a
  .lcsp from a newer peer only if it asked for one, so mixed versions still work.
- Measured with a debug kopanod on a busy PC (Firefox, other agents): edit latencies are 1.5 to
  3.4 s here against 0.9 to 1.6 s in the earlier release-build runs; S1 and S3 vary as much
  run to run.

**Smart Previews instead of JPEGs (2026-10-20, Light branch feature/kopano-smart-preview,
kopanod branch feature/kopanod-lcsp; screenshots in T:/kopano-lab/lcsp/run, fallback run in
T:/kopano-lab/lcsp/fallback)**

Same test, same 5 ARWs (246.5 MB), Member1's Light starts empty. Accuracy: Alice edits every
photo (exposure +2, highlights -60) on her RAW, the edit reaches Member1's preview, both windows
render at 1280 px (`ui.render`), mean absolute difference per channel out of 255.

| | Smart Preview (.lcsp) | JPEG fallback (owner on Light 0.5.0) |
|---|---|---|
| Moved to show 5 photos | 2.39 MB (1.0 %) | 3.33 MB (1.4 %) |
| Made on Alice's side | 0.7 to 1.0 s each, through the window | 2 to 10 s each, lightcraft-cli |
| First photo / all 5 after joining | 4.0 s / 11.5 s | 13.3 s / 26.2 s |
| As shot, vs the RAW | 1.12 (worst photo 1.79) | 0.73 (0.88) |
| Exposure +2, highlights -60, vs the RAW | **1.41** (worst 2.46; 0.69 on 16 px blocks) | **20.0** (worst 25.6) |
| Alice exposure to Member1 / Member1 contrast to Alice / stars | 1.8 s / 2.8 s / 0.05 s | 1.9 s / 2.4 s / 0.5 s |
| Original on request (49.4 MB) | 1.5 s transfer, 1.7 s swap; same Light photo id; exposure +1.25, contrast +40, 4 stars kept; still 5 photos | same edits kept; a new photo id |
| Edit on the original after the swap | 3.1 s | 1.9 s |
| Carol (valid certificate, not a member) asks for an original | refused at connect (`media: refused 6c1be93e`) | (not run) |
| End | same head; Member1 1 from original, 4 from preview | same |

Regressions on this branch: kopano-node unit tests 20/20 (1 new), kopano-lab Light tests 5/5
(1 new); S1 20/20 edits (median 533 ms), both ways, clashes settle, restart catch-up; S3 Light to
Light exposure 1.8 and 3.4 s, stars 3.7 and 2.1 s, slider drag 3.3 and 1.6 s (two runs), same
head.
After merging the kopano-core convergence fix (7270d90): unit tests 20/20, kopano-core 10/10
and 3/3; S1 20/20 (median 879 ms), all checks pass; S3 exposure 2.3 s, stars 2.0 s, slider 1.9 s,
same head; S5 again 2.39 MB, 4.5 s / 13.3 s, 1.41/255 edited, takeover with edits and the same
photo id, Carol refused. One S5 attempt before that stopped at the join ("that teammate is not
reachable", 12 s after both services started); the next run joined in 2.7 s.

## 7. Slice "photo": Ubunye Photo windows co-edit through kopanod

Before this slice the Photo connector (`kopano-lab/src/photo.rs`) only worked in a lab proof that
moved versions and tiles by hand. Now a running Photo window is bound to a project in kopanod like
Light is. Code: `lab/kopano-node/src/photo_binding.rs` (kept apart from `lib.rs` so the Film
binding can merge beside it); test: `lab/scenarios/s6_photo.py`.

**How it works**

1. Binding: `{"kind": "photo-control", "port": P, "token_file": F, "media_dir": optional}` in
   `create-project`, `join` and `attach`. The token file is read on every connect, because Photo
   writes a new token each launch. `photo-headless {cli}` binds a headless engine for tests.
2. Tiles are media. A tile item names its pixels by BLAKE3, which is also the iroh-blobs hash.
   Before a version is pushed, every tile and blob its items name is put in kopanod's media store
   (by reference, no second copy). A teammate fetches missing tiles over iroh-blobs through the
   same gate as Light media: only a current member's device, only hashes the project's versions
   name. Tiles are fetched 16 at a time on one connection, into memory and straight into the
   media folder (the file store synced the disk once per blob: 97 tiles took 9 to 55 s on this
   PC's hard disk; in memory 0.25 to 0.8 s).
3. A tile waiting for its pixels is not "undone". Until the pixels are here the window keeps the
   old tile, but saves see the project's tile, so nothing is sent back to the team. If the user
   repaints that tile meanwhile, the user's stroke wins.
4. One update per call. After every call the window gets one `kopano.import` (one undo step in
   Photo), built on the snapshot the call saved first. If Photo refuses it because the user
   edited after that snapshot, nothing half-applied is sent, what the call did to waiting tiles
   is undone, and kopanod saves the user's edit and brings the team's change in again (up to 8
   tries; a try that still fails waits for the next push). See "The refused-update data loss"
   below.
5. A window that (re)connects is brought to the project first (the S3b data-loss class). Its own
   state is kept as a version off the main line (`kept.json`), the project's items are applied
   and items the project does not have are taken out. A window that joins a project with no
   versions on this device yet says nothing until the first push, then becomes exactly the
   team's document. Only a lost connection reconnects; any other Photo error keeps the
   connection, so the user's unsaved edits are not moved off the main line.

**Two fixes outside Photo that this slice found (they help Light too)**

- `Node::integrate`: our own head coming back from a teammate went through `Repo::integrate`,
  whose first step saved the app's newest edit into the head without pushing it. A slider change
  then waited about 30 s for the next edit. Now a head we already contain is ignored, and a head
  moved by that first save is pushed.
- `Repo::integrate` (kopano-core): after a criss-cross (both sides merged each other at the same
  moment) there are two best common ancestors; the old breadth-first pick took the wrong one and
  reported work both sides already shared as clashes. S6 saw 10 false tile clashes per run. Now
  every best common ancestor (`project::merge_bases`) is tried and the merge with the fewest
  clashes is kept; the lane check of a merge accepts any of them. Test:
  `repo::criss_cross` (fails with the old pick).

**Real test (2026-10-20, this PC, two Photo windows, two kopanod with their own homes and API
ports, the owner's DSC08379 as a 3000x2000 document; screenshots in
T:/kopano-lab/photo-kopanod/side-by-side)**

| Step | Result |
|---|---|
| Alice starts the project from her window | 98 items (96 tiles), 2.4 s |
| Member1's window starts on a blank 800x600 document and joins | became Alice's document 1.7 s after the join returned; 97 tiles fetched in 0.25 s |
| Alice adds an Exposure layer, moves its slider | Member1 follows in 1.6 s and 1.5 s |
| Alice adds a layer, paints one stroke (4 tiles of 100) | 1.6 s, 1.1 s |
| Member1 paints a stroke | Alice follows in 1.4 s |
| Both paint at once, different tiles of one layer | settled in 2.3 s, 0 clashes |
| Member1 paints 207 strokes while Alice moves Exposure 4 times | settled in 1.9 s, 0 of Member1's tiles lost, Exposure -0.4 in both |
| Member1's Photo is closed and opened again on the untouched original | team edits back in 3.9 s; Alice's window and head unchanged; the window's own state kept as 1 version |
| The reopened window paints | Alice follows in 2.4 s |
| End | same head, windows match, 0 clashes, 0 media refusals |

One earlier run hit the refused-update path once (Photo refused, kopanod saved the stroke and
tried again; windows matched). Regressions: kopano-node unit tests 11/11 (5 new), kopano-core
9/9 plus 3 (1 new), S1 20/20 edits both ways, concurrent edits settle, restart catch-up
(median 953 ms on a loaded PC; the build before this slice measured 935 ms in the same
conditions).

**Limits found (honest)**

- Tiles travelled raw (256 KB each, about 25 MB per join). Fixed below ("Tiles travel packed").
- Photo's `kopano.objects` serves the latest export only, so while someone paints fast an
  export and its object fetch can disagree ("Photo has no object"). The connector exports again
  (up to 12 times; 11 to 15 times per S6 run, each a few ms). Serving the previous export too
  was tried and measured, and is NOT the fix: see "Tiles travel packed".
- kopanod restarting while Photo stays open treats the open window like a fresh one: edits made
  while kopanod was down go to `kept.json`, not to the team.
- The document's tab name and file name stay local ("Blank" in Member1's window).
- Layers made in a window before any update from the team get random uids that the window does
  not store (only an update writes the uid map into the document); a reconnect then sees them as
  new layers, keeps the old ones in `kept.json` and brings the project's back.

### Tiles travel packed (2026-10-20)

Code: `photo_binding.rs` (publish, `packed_offer`, `fetch_tiles`, `fetch_one`), the
`tiles-packed` request in `lib.rs`, `pack`/`unpack`/`keep_packed` in `kopano-lab/src/photo.rs`.
media.rs is unchanged: its gate already asks `photo_binding::serves`.

- Items do not change. A tile item still names its raw pixels by BLAKE3, so merges, clashes and
  older peers see the same identity.
- The connector keeps the zstd bytes Photo hands it (`media/zst/<raw>.zst`): packing costs
  nothing. A tile that came in raw is packed once when it is published.
- The node offers each packed blob under its own hash. Before fetching, a receiver asks the
  teammate `{"op": "tiles-packed", "hashes": [...]}` on the member-checked sync channel (in
  parallel with opening the blobs connection) and gets raw hash -> packed hash.
- Integrity: iroh-blobs checks the packed hash on the way in; the receiver unpacks (at most
  1 GiB) and checks the RAW hash from the signed version before the tile is written. The answer
  needs no signature of its own: a wrong one costs one wasted fetch, then that tile is fetched
  raw. Hashes in the answer are read defensively (iroh's own parser panics on some bad lengths).
- Membership: the gate serves a packed blob exactly when it would serve its raw object to that
  device.
- Older peers: an older kopanod refuses the question and everything is fetched raw; an older
  receiver never asks. `KOPANO_PHOTO_RAW_TILES=1` turns packing off (the "before" below).

**Real test (S6, this PC, two Photo windows, two kopanod, the owner's DSC08379 as a 3000x2000
document; screenshots T:/kopano-lab/photo-tiles/shots; runs T:/kopano-lab/photo-tiles)**

| | raw (before) | packed (after) |
|---|---|---|
| Join: moved for 97 tiles (25.2 MB of pixels) | 25.26 MB | 11.59 MB |
| Alice's stroke, 4 tiles, to Member1 | 1.05 MB | 0.06 MB |
| Member1's stroke, 6 tiles, to Alice | 1.58 MB | 0.05 MB |
| Whole run, both sides received | 41.3 MB | 11.8 MB |
| Join: window became Alice's document | 1.8, 2.0 s | 1.45 to 1.72 s (5 of 6 runs) |
| Stroke Alice -> Member1 / Member1 -> Alice | 1.0 s / 1.1 to 1.2 s | 1.0 to 1.3 s / 1.1 to 1.4 s |
| Both paint at once | 0 clashes | 0 clashes, settled 1.1 to 2.0 s |
| Member1 paints ~200 strokes while Alice moves Exposure | 0 tiles lost | 0 tiles lost (6 runs) |
| Member1's Photo closed and reopened | team edits back 4.6 to 4.8 s | 3.9 to 4.8 s, Alice unchanged |

One packed run's join took 5.6 s: the `tiles-packed` question waited 2.5 s for its connection
(every other run: under 10 ms). Packed tiles unpack in about 3 ms each. Mixed versions: an older
kopanod as Alice (sender) or as Member1 (receiver) gives the raw numbers, same head, windows
match, 0 clashes, 0 refusals.

**Serving Photo's previous export was tried and reverted.** With `kopano.objects` also serving
the export before the newest (Photo fork branch `feature/kopano-prev-export`, commit 50570b72,
reverted in d2a3ad2f), the connector no longer exports again on "no object" and reads a
snapshot one stroke old while the user paints. Its update is then refused by Photo every time:
6 to 7 refused updates per S6 run instead of 0. And the refused-update path lost tiles: in
step 5, 10 or 11 of Member1's tiles were replaced by older versions in both windows (same head,
0 clashes) in 6 of 25 runs, with the old kopanod (1 of 9) and the new one alike. With the
unchanged Photo the refused path is rare (0 refused updates in 10 runs here) and nothing was
lost. That loss is found and fixed (next section).

### The refused-update data loss (found and fixed 2026-10-20)

**Cause.** `Repo::integrate` (kopano-core) first saved the app's snapshot, then `apply_tree`
read the app a SECOND time and applied the merged tree over that second read. A stroke made
between the two reads was in the second read but not in the merge, so its tiles were put back
to their older versions. Photo accepted that update, because it was built on Photo's newest
revision. Nothing was ever saved with the stroke in it: same head on both sides, 0 clashes,
the stroke gone in both windows. A Photo that reads one stroke old (the previous-export build)
widens the gap between the two reads, which is why it lost more often. Refusals were the
symptom, not the cause: the update that should have been refused was the one accepted. The
same code path serves Light and Film (Light has no revision check, so a slider moved between
the two reads was put back too).

**Fix.** `Repo::save_seen` hands back the snapshot it saved, and integrate and restore apply
the target onto exactly that snapshot. Photo now refuses any update built on an older snapshot
than its document, and `Node::integrate` saves the edit and tries again. Two smaller holes of
the same class were closed with it:

- Photo binding: after a refused call, `Live::rewind` puts back the waiting tiles and the
  joining-window flag as they were before the call. Before, a tile applied from "waiting" was
  then neither waiting nor in the window (the next save would read the old tile as the user's),
  and a joining window whose first update was refused would speak first with its own document.
- Film binding: a deferred update (Film refused it, or the footage is on its way) now records
  what the window held. If the user edits that item, the user's edit wins and is saved. Before,
  the deferred update hid it from every save and the next retry undid it.

**Test that fails on the old code.** `sim_fuzz::sim_refused_updates_lose_nothing`: two windows
that behave like Photo (lazy model, one update per call, refused when the document moved since
the snapshot), Member1 painting and Alice moving a slider between any two calls, kopanod's
retry loop. Old code: 144 of 200 seeds lost a user's last edit (296 keys). Fixed: 0 of 500
seeds (11 850 refused updates). Unit tests: `a_refused_call_leaves_pending_tiles_and_a_joining_window_as_they_were`,
`a_user_edit_on_a_waiting_update_wins`.

**Real test (S6, this PC, owner's DSC08379 document; runs in T:/kopano-lab/refused)**

| | refusal-provoking Photo (50570b72) | normal Photo (d2a3ad2f) |
|---|---|---|
| Old kopanod, earlier runs | tiles lost in 6 of 25 runs (10 or 11 each) | lost in 1 of 9 |
| Old kopanod, 10 runs today | 0 lost, 7 refused updates per run (10 of 10) | |
| Fixed kopanod | 25 runs: 0 tiles lost, 8 refused updates per run, settled 2.1 to 2.8 s | 10 runs: 0 lost, 0 refused, settled 1.0 to 1.3 s |
| Fixed kopanod, whole run | same head, windows match, 0 clashes, reopen keeps the team's edits | same |

The fixed kopanod sees exactly one more refusal per run than the old one, in every run: that is
the update the old code accepted on its second read. The S6 count of lost tiles also
undercounts: a reverted tile that a later stroke paints over looks right in both windows,
although the stroke under it is gone. One fixed run (of 26) failed before step 5 at the join
("that teammate is not reachable", the address lookup) and was run again.

Also after the fix: kopano-node 23 of 23, kopano-core 10 of 10 plus 3, kopano-lab tests all
pass (sim_fuzz 500 seeds: 0 diverged, 0 stuck, 0 lost), S1 20 of 20 edits (median 125 ms),
concurrent edits settle, restart catch-up.

**Limit.** While a user paints without a pause, a team change waits for a gap: in S6 one
integrate was refused 8 times and landed on the 9th try, 5.4 s after it arrived. Nothing is
lost meanwhile, and a try that still fails waits for the next push.

## 8. Slice "Film": one film cut from two computers

The Film connector (E40, `lab/kopano-lab/src/film.rs`) now runs inside kopanod. Code:
`lab/kopano-node/src/film_binding.rs` (lib.rs only gains the binding and three hooks); test:
`lab/scenarios/s6_film.py`.

**How it works**

1. Binding `{"kind": "film-control", "port": P}` (the running Film, `filmcraft --control P`) or
   `{"kind": "film-headless", "cli", "project"}` for tests; `create-project`, `join` and `attach`
   take it like Light's.
2. Id blocks. kopanod gives each window it links an id block derived from the device, the
   project and a counter (`kopano.idBlock {block}`); Film refuses a block any id of the project
   already uses and kopanod takes the next. Two people never make the same clip id.
3. A window never speaks first. A window kopanod (re)connects to, or one that opened another
   file, gets its block first, then is brought to the head: its own state is kept off the main
   line (`kept.json`), the head is applied and clips the head does not have are taken out.
   Every snapshot is checked against the window's block (read in the same call as the items),
   so a file opened meanwhile is never saved as an edit. A window with nothing of this project
   open (Film just started, or another project) is left alone and shows the project's state.
4. Footage not here is not a change. A clip whose original has no media item in the window is
   the project's clip; an update Film refuses waits (`deferred`) and is tried again every few
   seconds and when footage arrives.
5. Footage through media.rs, membership-checked. A teammate's missing media item is recognised
   by name and size (asked from the owner's device), then gets a proxy: Film's own
   `media.createProxies` (H.264, quarter size) in a headless Film engine on the device with the
   original (made ahead for shared projects), attached with `media.attachProxies`. Originals on
   request (`fetch-original {key}`: a media hash, a clip key or `all`); the media item is then
   relinked, so exports are made from originals.
6. Flush: after team changes land, kopanod saves Film's project file (`file.save`, only when the
   window has one).

**Real test (2026-10-20, this PC, two Film windows, two kopanod with their own KOPANO_HOME and
KOPANO_API_PORT, the owner's 3 Sony XAVC clips; screenshots in T:/kopano-lab/film-kopanod)**

| Step | Result |
|---|---|
| Member1's project file points at footage that is not on his side | 3 of 3 clips play from proxies 11.5 s after joining: 1.34 MB for 71.3 MB of originals (1.9 %); proxies made on Alice's side in 0.9 to 3.3 s each |
| Id blocks | 103940031 and 484757726; a razor cut on each side at the same moment: 4 new clips, 2 from each block, all unique, both sides 10 clips in 3.7 s |
| Alice trims 15 frames and grades (Lumetri exposure, temperature, saturation, contrast) while Member1 sets -9 dB | in Member1's window 0.1 s after the edit commands returned, the level in Alice's 0.05 s; 0 clashes |
| Member1 asks for the 3 originals | 4.4 s, media items relinked |
| Same frame, same export | frame PNG identical; 2 s software H.264 export identical (6.48 MB) |
| Member1's Film closed; a fresh window runs empty 8 s, then opens the OLD file | team work untouched while empty; brought to the project in 0.2 s; old state kept as a version; Alice keeps grade, level and 10 clips; head unchanged |
| Alice starts an unrelated new project in her window | left alone; Member1 keeps everything |
| Alice reopens the old file | brought back in 3.6 s; kept as a version |
| 20 s quiet | both heads equal and still |
| Regressions | kopanod unit tests 14/14; S1 20/20 edits (median 560 ms, debug build on a busy PC), both ways, clashes settle, restart catch-up; S3 Light exposure 1.6 s, stars 1.5 s, slider drag 0.9 s, same head |

**Found and fixed on the way (each was data loss or a loop on the real app)**

- A fresh window that opened the old file between being brought to the project and getting its
  id block was saved as an edit and pushed (the team lost trim, grade and level). Fixed by the
  block-first order and the per-snapshot block check.
- An empty window read as "every clip removed" (a whole timeline lost). Fixed: no opinion shows
  the project's state.
- `integrate` ("same content, settled") picked the smaller id even when that version was older:
  two forks whose trees cross-matched cycled for ever and the window flipped between two states
  every second. Settling now moves only forward (versions behind it, then the smaller id).

**Open**

- New sequences and project items do not travel yet (E40): both sides need the project file.
- A clip arriving from a teammate is not checked for overlap; a user edit made while a merge is
  half applied can be refused by Film for an overlap (seen once in testing; nothing lost).
- When a window lacks the media item entirely, the proxy is imported as the item and relinked to
  the original later; Film may keep the proxy's frame size in the item info (not tested).
- Film shows "Media missing" with a P badge on proxy clips and asks to link footage when the
  file opens; the panel should say the footage is on its way.

## 9. Slice "approve": the owner's line moves only when she approves

Before this slice kopanod ran one shared live line: every edit went to everyone. The core already
had the owner's v2 model (`kopano-core/src/workflow.rs`: main owned by the creator, one branch
per member, signed ref updates, approvers, members log, seal after members changes, forks offered
again), tested only in the simulator. Now kopanod runs it. Code: `lab/kopano-node/src/approve.rs`
(lib.rs gains the hooks); core: `Genesis.mode`, `workflow::review_of`, `workflow::approve_with`;
test: `lab/scenarios/s7_approve.py`. Light panel: branch `feature/kopano-approve`
(`crates/ui-egui/src/ubunye/kopano.rs`, `kopano_panel.rs`).

**How it works**

1. The mode is chosen when sharing (`create-project {mode: "live" | "approve"}`; the API and the
   panel default to "approve" for new projects). It is written into the owner-signed genesis, so
   every member reads the same mode. A live project writes no mode field, so its genesis bytes
   and id stay what they were: every existing project stays live.
2. What each window shows is the project's `head` on that device. The owner's window shows main
   and her edits go straight to main (a signed update of `main`). A member's edits go to
   `branch/<account>`; his window shows his branch with the latest main merged in, or main itself
   when nothing of his waits. He sees his own edits at once.
3. Sync is the same watch/push channel. In approve mode a push also carries the ref updates and
   members entries the watcher lacks (kept in `refs.json` and `members.json`, objects stored
   before the files that name them). A receiver checks every ref (`workflow::check_ref` and the
   chain rules) and the history under it: under an approver's main only signatures and
   certificates (the approver vouches), under a branch the author's lanes by the members the
   update cites. Membership (sync and media gate) comes from the genesis plus the signed members
   log; media still flows by membership as before.
4. API: `pending` (per member: photos changed since the main they built on, what changed per
   photo, clashes with main with both values), `approve {member, version, prefer, choices}`
   (a merge into main, signed; each clash decided per photo or item: "member" or "main"),
   `decline {member, version}` (an approver-signed `review/<account>` mark; main stays, the
   branch keeps the work), `view {member, show: member | main | off}` (the owner's window shows
   the member's version of the photos he changed, or hers; her saves pause until "off", so
   looking never reaches main), `set-approver {account, approver}` (a members entry, then main is
   sealed). `status` gains `mode` and `review`; events `review` tell the panel what changed. A
   member who is not an approver is refused locally, and a main update he signs anyway is refused
   by every receiver.
5. Declined stays declined when main moves on: a branch whose only new versions since the
   declined one are catch-up merges is not offered for review again; the member's next edit is.
6. Live projects also gain a clash pick: `status.clashes` lists open clashes with both values of
   the clashing setting, `resolve {clash, pick: a | b}` writes that value (other settings stay
   as merged), removes the clash record and sends it to everyone.

**Panel (Light).** Share offers "I approve changes" (default) and "Work together live". The owner
sees "Changes to review": "Member1 changed 1 photo  View  Approve  Decline"; View turns into
Before / After / Done; a clash shows both values with "Keep yours" / "Take theirs". The member sees
"Alice approves changes" and "You changed 1 photo · Waiting for Alice's approval", then
"Approved: Alice took your changes" or "Declined: Alice kept the library as it was. Your changes
stay in your window." A live library shows its clashes with one button per value. No protocol
name on screen.

**Real test (2026-10-20, this PC, two Light windows (release build) each with its own kopanod
(debug build, own KOPANO_HOME and KOPANO_API_PORT), the owner's 5 Sony ARWs in both libraries,
panels open; screenshots of both windows at every step in T:/kopano-lab/approve)**

| Step | Result |
|---|---|
| Alice shares in approve mode, Member1 joins | 15 items, 5 photos; his window shows main 0.8 s after the join returned |
| Member1: exposure +1.5 and 4 stars | his window at once; Alice's main and window unchanged (exposure 0, 0 stars); her panel "Member1 changed 1 photo" after 0.7 s; his "Waiting for Alice's approval" |
| Alice presses View, Before, Done | her window shows his +1.5 (0.5 s), then her 0; main unchanged, nothing saved |
| Alice approves | her window has +1.5 and 4 stars 0.4 s after the command; his panel "Approved"; his window is main again |
| Member1: contrast +30, Alice declines | her main and window unchanged; his window keeps +30; his panel "Declined" |
| Alice edits another photo on main (exposure -0.7) | in his window 0.7 s (3.4 s in the run before); his declined +30 stays; still "Declined", nothing to review |
| Both kopanod stopped and started | heads, review state ("Declined") and windows kept; his next edit (contrast 35) waits for review again |
| Member1 presses Approve on his own work | refused ("Only the owner, or someone the owner made an approver, can approve changes"), also on his panel; main unchanged on both |
| Checks | 30 of 30 in two runs in a row (the first run, 29 of 29, found the declined work offered again after main moved; fixed and added as a check and a unit test) |

Regressions on this branch: kopano-core 10 of 10 plus 3; kopano-node 33 of 33 (10 new: approve, decline, owner edits reach a member with work waiting, clashes per photo, a non-approver and a forged main update refused, approver granted and taken back, view saves nothing, restart keeps state, live genesis bytes unchanged, live clash pick); Light panel tests 24 of 24 (4 new), clippy and fmt clean; sim_workflow 200 of 200; attacks 4 of 4; sim_fuzz 200 seeds, 0 diverged, 0 stuck, 0 silent losses, refused updates 200 seeds 0 lost; S1 20 of 20 edits (median 936 ms, debug build, busy PC), both ways, concurrent edits settle, restart catch-up; S3 in live mode (now asked for explicitly) exposure 0.8 s, stars 0.9 s, slider drag 1.0 s, same head.

**Limits found (honest)**

- Tested end to end with Light only. Photo and Film windows sign their saves into the branch too
  (the same hook), but no real Photo or Film run in approve mode yet.
- The owner's edits in her window while she views a member's version are not saved (saves pause
  until Done; the panel says so).
- Names come from certificates seen (handshake, genesis, ref updates); until one is seen, the panel
  shows the account.
- One approver in the real test: two approvers approving at once (a main fork, offered again) is
  covered by sim_workflow and the code path, not by a real run. Restarting the Light windows
  themselves (not only kopanod) was not part of S7.
- `pending` diffs every waiting branch on each status poll; fine for a shoot, not measured at
  thousands of photos.
- Older panels that send no mode now create approve projects (the API default; scripts that
  want live say `mode: "live"`, as S3 now does). A kopanod from before this slice refuses an
  approve project's genesis (its signature no longer checks without the mode field), so it can
  not join one by mistake. Adding and removing members and blocking a device have no API yet
  (only `set-approver`).
