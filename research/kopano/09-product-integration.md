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
   teammate (owner's devices first) for a preview, fetches it, and imports it into Light. The
   device with the original makes each preview once, with Light's own renderer
   (`lightcraft-cli render`, 2560 px JPEG, sharpening and noise reduction off, as shot), keeps it,
   and makes them ahead of time for shared projects so a joiner does not wait for renders.
   A device that only has the preview can hand it on.
3. The key. Light keys a photo by the content hash of the file it imported. A preview is a
   different file, so kopanod tells the Light connector "this preview file is the photo with the
   ORIGINAL's key" (an alias by file path). The connector reports the photo under that key, with
   the original's asset reference. Rules: a file in kopanod's preview folder without an alias is
   never published as a photo; once the original is in Light, the original wins over its preview.
4. Waiting photos are not deletions. A photo whose preview has not arrived still counts as the
   project's state (the core would otherwise read an empty window as "everything removed" and
   send that to the team). Changes for it wait in the project and are applied on arrival.
5. Original on request: API `fetch-original {project, key}`. The size is proven first and must fit
   on the disk, a cut download continues where it stopped (iroh-blobs keeps what arrived), and
   `original` events report MB done every second. The original is imported beside the preview,
   given the project's settings, and the preview is taken out of Light.
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

- A preview is a rendered JPEG: as shot it matches the RAW within 0.8/255 (mean), contrast 1,
  white balance 2, but exposure +-2 is off by 15-36/255 and highlights/shadows by 27-31/255
  (measured in Light at 1280 px). It is a picture to work on, not to judge tones on; the final
  export happens where the originals are. E13 showed Light's own Smart Preview stays within
  0.5/255, but Light cannot take one in today (see below).
- The preview photo shows the file name `DSC08379.jpg` and kind "image" until the original comes.
- The member's Light imports previews "in place" from kopanod's media folder; a Light restarted
  in memory mode gets them back from that folder without the network.

**Light changes that would make previews exact (not made: Light repo untouched)**

1. `library.addFromSmartPreview {smartPreview: path to .lcsp, contentHash, fileName, captured,
   fileSize, width, height, kind}`: a photo whose original is offline, keyed by the original's
   content hash, rendered from the Smart Preview. No alias needed, edits match within 0.5/255.
2. Smart Previews in `--memory` libraries (today they need a library on disk).
3. `photo.exportSmartPreview {id, path}` on the control port, so kopanod does not need
   `lightcraft-cli` beside it to make previews.
4. `photo.relink` keeps the old kind and format (a JPEG relinked to an ARW stays "image"), which is
   why the original is imported beside the preview instead of relinked.

