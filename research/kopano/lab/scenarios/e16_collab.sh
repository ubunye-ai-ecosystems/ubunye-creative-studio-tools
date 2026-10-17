#!/usr/bin/env bash
# E16: collaboration scenarios on real devices. Alice = this PC (owner), Bob = Soshanguve laptop,
# Member0 = home laptop (same network as this PC). Carol = valid account, not a member (this PC).
# Usage: e16_collab.sh <bin dir> <project root> <home agent id> <soshanguve agent id>
set -u
BIN="$1"; R="$2"
P="$BIN/peer.exe"; A="$BIN/agent.exe"
declare -A ID=( [home]="$3" [sosh]="$4" )
alice_pid=""

say() { echo "{\"scenario\":\"$1\",\"who\":\"${2:-}\",\"out\":${3:-null}}"; }
call() {  # laptop, json (retries while a helper is being found)
  local o="" n
  for n in 1 2 3; do
    o=$(timeout 900 "$A" call --dir "$R/alice" --to "${ID[$1]}" --json "$2" 2>&1 | grep '^{' | tail -1)
    [ -n "$o" ] && break
    sleep 5
  done
  echo "${o:-{\"error\":\"no reply\"\}}"
}
alice_stop() { [ -n "$alice_pid" ] && taskkill //F //PID "$alice_pid" > /dev/null 2>&1; alice_pid=""; sleep 1; }
alice_serve() {
  alice_stop; rm -f "$R/alice/addr.json"
  # This PC's server sits on its own relay, never the one the instruction tool uses (same identity).
  KOPANO_RELAY_URL="${ALICE_RELAY:-}" "$P" serve --dir "$R/alice" > "$R/alice-serve.log" 2>&1 &
  until [ -s "$R/alice/addr.json" ]; do sleep 0.3; done
  alice_pid=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter \"Name='peer.exe'\" | Where-Object { \$_.CommandLine -like '*serve*e15*alice*' } | Select-Object -First 1).ProcessId" | tr -d '\r')
}
alice_edit() { alice_stop; "$P" edit --dir "$R/alice" --key "$1" --field "$2" --value "$3" | grep '^{'; }
alice_pull() { alice_stop; "$P" pull --dir "$R/alice" --from "$1" --closure 2>&1 | grep -E '^\{|^Error' | tail -1 | python -c "import sys,json;t=sys.stdin.read().strip();print(t if t.startswith('{') else json.dumps({'error':t}))"; }
addr_json() { python -c "import json,sys;print(json.dumps(open(sys.argv[1]).read()))" "$1"; }
edit() { call "$1" "{\"op\":\"run\",\"args\":[\"edit\",\"--key\",\"$2\",\"--field\",\"$3\",\"--value\",\"$4\"]}"; }
laptop_addr() { call "$1" '{"op":"serve-addr"}' | python -c "import json,sys;d=json.loads(sys.stdin.read());open(sys.argv[1],'w').write(d['reply']['result']['addr'])" "$R/$1-addr.json"; }
pull_from_alice() {  # laptop [--relay-only]
  local a; a=$(addr_json "$R/alice/addr.json")
  call "$1" "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"alice-addr.json\",\"--closure\"${2:+,\"$2\"}],\"files\":{\"alice-addr.json\":$a}}"
}
pull_between() {  # to from [--relay-only]
  laptop_addr "$2"
  call "$1" "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"peer-addr.json\",\"--closure\"${3:+,\"$3\"}],\"files\":{\"peer-addr.json\":$(addr_json "$R/$2-addr.json")}}"
}
# Fingerprint of everyone's photo settings (all 1,003 items + open clashes), to prove they match.
state_of_laptop() {
  call "$1" '{"op":"run","args":["status"]}' | python -c "
import sys,json,hashlib
d=json.loads(sys.stdin.read()); so=d['reply']['result']['stdout']
items=sorted(l.strip() for l in so.splitlines() if l.strip().startswith(('item ','CLASH ')))
print(json.dumps({'items':len(items),'clashes':sum(1 for l in items if l.startswith('CLASH')),'fingerprint':hashlib.sha256('\n'.join(items).encode()).hexdigest()[:16]}))"
}
state_of_alice() {
  alice_stop; "$P" status --dir "$R/alice" | python -c "
import sys,json,hashlib
items=sorted(l.strip() for l in sys.stdin.read().splitlines() if l.strip().startswith(('item ','CLASH ')))
print(json.dumps({'items':len(items),'clashes':sum(1 for l in items if l.startswith('CLASH')),'fingerprint':hashlib.sha256('\n'.join(items).encode()).hexdigest()[:16]}))"
}

H=$("$P" hash-of --dir "$R/alice" --key sony-a7m4-uncompressed.ARW | tr -d '\r')

echo "### TWO COLLABORATORS: this PC (owner) + Soshanguve"
say "2.1 hello" sosh "$(call sosh '{"op":"info"}')"
alice_serve
say "2.1 Soshanguve joins" sosh "$(pull_from_alice sosh)"

alice_edit "canon-5d4-raw.CR2" develop.exposure 0.6 > /dev/null; alice_serve
say "2.2 owner edit reaches Soshanguve" sosh "$(pull_from_alice sosh)"

say "2.3 Soshanguve edits" sosh "$(edit sosh IMG_0000010.ARW develop.contrast 25)"
laptop_addr sosh
say "2.3 owner gets Soshanguve's edit" alice "$(alice_pull "$R/sosh-addr.json")"

alice_edit "IMG_0000020.ARW" develop.exposure -0.3 > /dev/null
say "2.4 Soshanguve edits a different photo" sosh "$(edit sosh IMG_0000030.ARW develop.exposure 0.8)"
laptop_addr sosh
say "2.4 owner merges" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
say "2.4 Soshanguve gets the merge" sosh "$(pull_from_alice sosh)"

alice_edit "IMG_0000040.ARW" develop.exposure 1.2 > /dev/null
say "2.5 Soshanguve edits same photo, other setting" sosh "$(edit sosh IMG_0000040.ARW develop.contrast -15)"
laptop_addr sosh
say "2.5 owner merges (both kept)" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
say "2.5 Soshanguve gets the merge" sosh "$(pull_from_alice sosh)"

alice_edit "fuji-xt4-lossless.RAF" develop.exposure 0.9 > /dev/null
say "2.6 Soshanguve changes the same setting" sosh "$(edit sosh fuji-xt4-lossless.RAF develop.exposure -0.5)"
laptop_addr sosh
say "2.6 owner merges (clash expected)" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
say "2.6 Soshanguve gets the clash record" sosh "$(pull_from_alice sosh)"

for k in 1 2 3 4 5; do alice_edit "IMG_00001$k.ARW" develop.temperature "$((5000 + k * 100))" > /dev/null; done
alice_serve
say "2.7 Soshanguve catches up on 5 edits at once" sosh "$(pull_from_alice sosh)"

laptop_addr sosh
say "2.8 outsider (Carol) tries to pull from Soshanguve" carol "$("$P" pull --dir "$R/carol" --from "$R/sosh-addr.json" --closure 2>&1 | grep -E '^\{|Error|refused|not a member' | head -2 | python -c "import sys,json;print(json.dumps({'output':sys.stdin.read().strip()[:300]}))")"

say "2.9 Soshanguve fetches a 34 MB original" sosh "$(call sosh "{\"op\":\"run\",\"args\":[\"fetch-media\",\"--from\",\"alice-addr.json\",\"--hash\",\"$H\"],\"files\":{\"alice-addr.json\":$(addr_json "$R/alice/addr.json")},\"timeout_s\":1500}")"

say "2.x state: owner" alice "$(state_of_alice)"
say "2.x state: Soshanguve" sosh "$(state_of_laptop sosh)"

echo "### THREE COLLABORATORS: + home laptop"
alice_serve
say "3.1 home laptop joins" home "$(pull_from_alice home)"
alice_stop
say "3.2 Soshanguve edits while the owner is offline" sosh "$(edit sosh IMG_0000050.ARW develop.exposure 0.4)"
say "3.2 home gets it straight from Soshanguve" home "$(pull_between home sosh)"

alice_edit "IMG_0000060.ARW" develop.exposure 0.1 > /dev/null
say "3.3 home edits" home "$(edit home IMG_0000061.ARW develop.contrast 7)"
say "3.3 Soshanguve edits" sosh "$(edit sosh IMG_0000062.ARW develop.temperature 6200)"
say "3.3 ring: Soshanguve <- home" sosh "$(pull_between sosh home)"
alice_serve
say "3.3 ring: home <- owner" home "$(pull_from_alice home)"
laptop_addr home
say "3.3 ring: owner <- home" alice "$(alice_pull "$R/home-addr.json")"
alice_serve
say "3.3 ring: Soshanguve <- owner" sosh "$(pull_from_alice sosh)"
say "3.3 ring: home <- owner (final)" home "$(pull_from_alice home)"

alice_edit "canon-5d4-raw.CR2" develop.temperature 4800 > /dev/null
say "3.4 home sets the same setting" home "$(edit home canon-5d4-raw.CR2 develop.temperature 5600)"
say "3.4 Soshanguve sets the same setting" sosh "$(edit sosh canon-5d4-raw.CR2 develop.temperature 7000)"
say "3.4 home <- Soshanguve" home "$(pull_between home sosh)"
alice_serve
say "3.4 owner <- home" alice "$(laptop_addr home; alice_pull "$R/home-addr.json")"
alice_serve
say "3.4 Soshanguve <- owner" sosh "$(pull_from_alice sosh)"
say "3.4 home <- owner" home "$(pull_from_alice home)"

alice_stop
say "3.5 home fetches the original from Soshanguve (not from the owner)" home "$(laptop_addr sosh; call home "{\"op\":\"run\",\"args\":[\"fetch-media\",\"--from\",\"peer-addr.json\",\"--hash\",\"$H\"],\"files\":{\"peer-addr.json\":$(addr_json "$R/sosh-addr.json")},\"timeout_s\":1500}")"

say "3.6 Soshanguve edits" sosh "$(edit sosh IMG_0000070.ARW develop.exposure -1.0)"
say "3.6 home <- Soshanguve, relay only" home "$(pull_between home sosh --relay-only)"

laptop_addr sosh
say "3.7 final sync: owner <- Soshanguve" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
say "3.7 final sync: home <- owner" home "$(pull_from_alice home)"
say "3.7 final sync: Soshanguve <- owner" sosh "$(pull_from_alice sosh)"

say "3.x state: owner" alice "$(state_of_alice)"
say "3.x state: Soshanguve" sosh "$(state_of_laptop sosh)"
say "3.x state: home" home "$(state_of_laptop home)"
