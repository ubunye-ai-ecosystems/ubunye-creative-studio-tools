# kopanod local API (what the Light, Photo and Film panels call)

kopanod listens on `127.0.0.1:<port>` (`KOPANO_API_PORT`, default 47100). One JSON object per line
in, one per line out. Every request carries the token from `<home>/api.token` (readable by this
user only):

```json
{"op": "hello", "token": "<api.token>"}
```

Answers: `{"ok": true, "result": ...}` or `{"ok": false, "error": "<a sentence to show as is>"}`.
Error sentences are plain English for the person, never protocol words, never emails.

`{"op": "subscribe"}` keeps the connection open and streams events, one JSON per line.

Home folder: `KOPANO_HOME`, else `%LOCALAPPDATA%\Kopano` (Windows), `~/Library/Application
Support/Kopano` (macOS), `~/.local/share/kopano` (Linux).

ThabangVision's address: `KOPANO_TV_URL`, else `<home>/server.json` `{"url": "..."}`, else the local
test server `http://localhost:3100`. The production site is refused unless
`KOPANO_TV_PRODUCTION=1` (it waits for the `kopano` flag and the owner's approval). The contract
kopanod follows is `docs/kopano/desktop-api.md` in the ThabangVision repo.

## 1. Signed in or not

### `hello`

Works in both states. Signed in:

```json
{"device": "<64 hex>", "name": "Thabang", "account": "<uuid>", "signed_in": true,
 "signed_in_until_ms": 1763467200000, "signing_in": null, "message": null,
 "server": "http://localhost:3100", "projects": [{"id": "<64 hex>", "name": "...", "app": {...}}]}
```

Not signed in (a fresh install, or signed out): `signed_in: false`, `name` and `account` empty,
`projects: []`, and

- `signing_in`: `{"code": "BCDF-1234", "link": "<site>/link?code=BCDF-1234", "until_ms": ...}`
  while a sign-in waits for the person in the browser, else `null`;
- `message`: why the last sign-in ended or why this computer was signed out, else `null`. For
  example "Sign-in was not approved in the browser.", "The sign-in code ran out. Press Sign in
  again.", "This computer was unlinked from ThabangVision. Sign in again.".

While not signed in every other op answers "This computer is not signed in to ThabangVision yet."
and nothing goes online.

### `sign-in`

Starts "Sign in with ThabangVision" (or hands back the code already waiting) and opens the browser
at the link page. Result: `{"code": "BCDF-1234", "link": "...", "until_ms": ...}`. Show the code in
the panel ("Check that the browser shows BCDF-1234") with a link to open the page again. When the
person approves, kopanod goes online by itself: open connections close, the panel reconnects and
`hello` says `signed_in: true`. Set `KOPANO_NO_BROWSER=1` where the app opens the page itself.

When already signed in: error "This computer is already signed in."

### `sign-in-cancel`

Stops a waiting sign-in. Result `"not signing in"`.

### `sign-out`

Forgets this computer's sign-in and goes offline (the device key stays, so signing in again links
the same computer and keeps its projects). Open connections close; `hello` then says
`signed_in: false`. Event `{"event": "account", "signed_in": false, "message": null}` first.

### Renewal (no call)

Sign-ins last 30 days and renew by themselves when less than 7 days are left, and at start-up when
expired. Event `{"event": "account", "signed_in": true, "renewed": true, "signed_in_until_ms": ...}`.
If ThabangVision says the computer was unlinked or the account is suspended, kopanod signs out and
`hello.message` says why.

## 2. Projects with people (ThabangVision)

New projects (`create-project`, mode `approve` by default) can be shared. A project made in `live`
mode cannot take new people ("This project cannot take new people. Start a new project to share
it.").

### `share {project}`

Puts a project you own on ThabangVision. Result `{"shared": "<ThabangVision id>"}`. Calling it again
is harmless. `invite` shares first by itself, so a panel rarely needs this.

### `invite {project, email, role?}`

Invite by email. `role`: `editor` (default), `viewer`, `approver`, `admin` (admin: owner only).
Result always `{"message": "Invite sent."}` once you may invite, whether or not the address has an
account (no account enumeration). Errors: "Type the teammate's email address.", "Only the
project's owner can share it.", ThabangVision's own sentences ("Project not found.").

### `people {project}`

```json
{"shared": "<ThabangVision id>", "people": [
  {"user": "<uuid>", "name": "Thabang", "avatar": "https://..." , "role": "owner", "status": "active",
   "me": true, "online": true, "on_project": true}
]}
```

Display names only, never emails. `status`: `invited` (not joined yet) or `active`. `online`: a
computer of theirs is connected now. `on_project`: signed into the project's members list, so
their computers may open it (the owner's computer does this when they join). For a project that is
not shared: `{"shared": false, "people": [you]}`. When the owner opens this list, kopanod also
brings the members list up to date (someone left on the website).

### `shared-projects`

"My projects" and "Shared with me":

```json
{"mine":   [{"shared": "<id>", "name": "...", "project": "<64 hex>", "role": "owner", "status": "active", "here": true}],
 "shared": [{"shared": "<id>", "name": "...", "project": "<64 hex>", "role": "editor", "status": "invited", "here": false}]}
```

`here`: the project is on this computer already. Only `invited` and `active` rows are listed.

### `join-shared {shared, app?}`

Opens a project from "Shared with me" with nothing typed: `shared` is the `shared` id from the
list (the project id works too); `app` is the binding, as in `join`. kopanod tells ThabangVision
the person joined, asks which computers the project's people use (the owner's first) and joins
from the first one online. The owner's computer lets them in when they knock. Result
`{"project": "<64 hex>", "name": "..."}`. Waits up to a minute for the owner's computer. Errors:
"That project is not shared with you.", "The project owner's computer is not online. Try again
when it is.", "The owner's computer has not let you in yet. Try again in a minute.", "The project
owner's computer has not been signed in yet. Try again when it is."

### `leave {project}`

Leave a shared project (project id or `shared` id). Result `{"left": true}`. The project stops
syncing and is no longer opened on this computer (its folder stays, marked `left`). The owner
cannot leave ("You own this project, so you cannot leave it."). Event `{"event": "left", ...}`.

### `remove-person {project, user}`

Owner only: take someone off (`user` from `people`). Their computers are refused from then on.
Result `{"removed": true}`.

### Events

- `{"event": "people", "project"}`: the members list changed (someone joined or was taken off).
- `{"event": "joined", "project", "name"}`, `{"event": "left", "project"}`.
- `{"event": "account", ...}`: see section 1.

## 3. Projects by id (lab tests; unchanged)

`create-project {name, app?, members?, mode?}`, `join {project, from, app?}`, `attach {project,
app}`, `add-teammate {project, device}`, `status {project}`, `get`/`set {project, key, field,
value?}`, `items`, `media`, `fetch-original {project, key}`, `review`/`pending`, `approve {project,
member, version?, prefer?, choices?}`, `decline`, `view`, `set-approver {project, account,
approver}`, `clashes`, `resolve {project, clash, pick}`. Members given here by account id still
work; people invited by email are added to the same members list.
