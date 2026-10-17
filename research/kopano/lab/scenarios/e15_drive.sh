#!/usr/bin/env bash
# E15: drive the test helpers on two real laptops (home network, Soshanguve) from this PC (Alice).
# Usage: e15_drive.sh <bin dir with peer.exe + agent.exe> <project root> <home agent id> <soshanguve agent id> [relay url]
# Prints one JSON line per step.
set -u
BIN="$1"; R="$2"; HOME_ID="$3"; SOSH_ID="$4"; RELAY="${5:-}"
LAPTOPS="${LAPTOPS:-home soshanguve}"
P="$BIN/peer.exe"; A="$BIN/agent.exe"
declare -A ID=( [home]="$HOME_ID" [soshanguve]="$SOSH_ID" )
declare -A WHO=( [home]="m0" [soshanguve]="bob" )
alice_pid=""

say() { echo "{\"step\":\"$1\",\"laptop\":\"${2:-}\",\"out\":${3:-null}}"; }
call() {  # laptop, json; retries while a helper is still being found online
  local o="" n
  for n in 1 2 3; do
    o=$(timeout 900 "$A" call --dir "$R/alice" --to "${ID[$1]}" --json "$2" 2>&1 | grep '^{' | tail -1)
    [ -n "$o" ] && break
    sleep 5
  done
  echo "${o:-{\"error\":\"no reply after 3 tries\"\}}"
}
alice_stop() { [ -n "$alice_pid" ] && taskkill //F //PID "$alice_pid" > /dev/null 2>&1; alice_pid=""; sleep 1; }
alice_serve() {  # [--relay-only]
  alice_stop; rm -f "$R/alice/addr.json"
  if [ -n "$RELAY" ]; then export KOPANO_RELAY_URL="$RELAY"; fi
  "$P" serve --dir "$R/alice" ${1:-} > "$R/alice-serve.log" 2>&1 &
  until [ -s "$R/alice/addr.json" ]; do sleep 0.3; done
  alice_pid=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter \"Name='peer.exe'\" | Where-Object { \$_.CommandLine -like '*serve*e15*alice*' } | Select-Object -First 1).ProcessId" | tr -d '\r')
}
alice_edit() { alice_stop; "$P" edit --dir "$R/alice" --key "$1" --field "$2" --value "$3" | grep '^{'; }
addr_json() { python -c "import json,sys;print(json.dumps(open(sys.argv[1]).read()))" "$1"; }
pull_from_alice() {  # laptop, [--relay-only]
  local a; a=$(addr_json "$R/alice/addr.json")
  local env='{}'; [ -n "$RELAY" ] && env="{\"KOPANO_RELAY_URL\":\"$RELAY\"}"
  call "$1" "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"alice-addr.json\",\"--closure\"${2:+,\"$2\"}],\"files\":{\"alice-addr.json\":$a},\"env\":$env}"
}
laptop_addr() {  # laptop -> writes $R/<laptop>-addr.json
  call "$1" '{"op":"serve-addr"}' | python -c "import json,sys;d=json.loads(sys.stdin.read());open(sys.argv[1],'w').write(d['reply']['result']['addr'])" "$R/$1-addr.json"
}

# 1. hello: who answers, how the control connection got there
for l in $LAPTOPS; do say "1 hello" "$l" "$(call $l '{"op":"info"}')"; done

# the original's fingerprint, read while Alice's database is free
H=$("$P" hash-of --dir "$R/alice" --key sony-a7m4-uncompressed.ARW | tr -d '\r')

# 2. both laptops join (1,000 synthetic photos + 3 real ones, edit data only)
alice_serve
for l in $LAPTOPS; do say "2 join" "$l" "$(pull_from_alice $l)"; done

# 3. Alice edits, both laptops pull the edit
for k in 1 2 3; do
  alice_edit "canon-5d4-raw.CR2" develop.exposure "0.$k" > /dev/null
  alice_serve
  for l in $LAPTOPS; do say "3 alice edit -> laptop (run $k)" "$l" "$(pull_from_alice $l)"; done
done

# 4. a laptop edits, Alice pulls from it
alice_stop
declare -A KEY=( [home]="IMG_0000004.ARW" [soshanguve]="IMG_0000005.ARW" )
for l in $LAPTOPS; do
  say "4 laptop edits" "$l" "$(call $l "{\"op\":\"run\",\"args\":[\"edit\",\"--key\",\"${KEY[$l]}\",\"--field\",\"develop.contrast\",\"--value\",\"12\"]}")"
  laptop_addr "$l"
  say "4 alice pulls from laptop" "$l" "$("$P" pull --dir "$R/alice" --from "$R/$l-addr.json" --closure 2>&1 | grep '^{' | tail -1)"
done

CL=${LAPTOPS##* }   # the laptop used for the clash
if [ "$LAPTOPS" = "home soshanguve" ]; then
# 5. laptop to laptop: home pulls from Soshanguve and back
laptop_addr soshanguve; laptop_addr home
say "5 home pulls from soshanguve" home "$(call home "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"peer-addr.json\",\"--closure\"],\"files\":{\"peer-addr.json\":$(addr_json "$R/soshanguve-addr.json")}}")"
say "5 soshanguve pulls from home" soshanguve "$(call soshanguve "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"peer-addr.json\",\"--closure\"],\"files\":{\"peer-addr.json\":$(addr_json "$R/home-addr.json")}}")"

fi
# 6. clash: Alice and Soshanguve change the same setting while apart
alice_edit "sony-a7m4-uncompressed.ARW" develop.exposure 1.1 > /dev/null
say "6 $CL edits same setting" $CL "$(call $CL '{"op":"run","args":["edit","--key","sony-a7m4-uncompressed.ARW","--field","develop.exposure","--value","-0.7"]}')"
laptop_addr $CL
say "6 alice pulls (clash expected)" $CL "$("$P" pull --dir "$R/alice" --from "$R/$CL-addr.json" --closure 2>&1 | grep '^{' | tail -1)"
alice_serve
say "6 $CL pulls the merge" $CL "$(pull_from_alice $CL)"
say "6 $CL status" $CL "$(call $CL '{"op":"run","args":["status"]}')"

# 7. media: each laptop fetches one original (34 MB) from Alice
for l in $LAPTOPS; do
  a=$(addr_json "$R/alice/addr.json")
  say "7 fetch 34 MB original from alice" "$l" "$(call $l "{\"op\":\"run\",\"args\":[\"fetch-media\",\"--from\",\"alice-addr.json\",\"--hash\",\"$H\"],\"files\":{\"alice-addr.json\":$a},\"timeout_s\":1500}")"
done

# 8. same small sync, relay only (worst-case network)
alice_edit "fuji-xt4-lossless.RAF" develop.exposure 0.4 > /dev/null
alice_serve --relay-only
for l in $LAPTOPS; do say "8 relay-only pull" "$l" "$(pull_from_alice $l --relay-only)"; done
alice_stop
"$P" status --dir "$R/alice" | head -3
