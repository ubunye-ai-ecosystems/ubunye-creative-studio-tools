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
