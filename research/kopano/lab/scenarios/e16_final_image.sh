#!/usr/bin/env bash
# E16 final image: all three devices edit one photo, sync, and each renders its own final image.
# Usage: e16_final_image.sh <bin dir> <root> <home id> <sosh id> [alice relay url]
set -u
BIN="$1"; R="$2"; P="$BIN/peer.exe"; A="$BIN/agent.exe"
declare -A ID=( [home]="$3" [sosh]="$4" )
ALICE_RELAY="${5:-https://use1-1.relay.n0.iroh.link./}"
KEY=sony-a7m4-uncompressed.ARW
alice_pid=""
say() { echo "{\"step\":\"$1\",\"laptop\":\"${2:-}\",\"out\":${3:-null}}"; }
call() { local o="" n; for n in 1 2 3; do o=$(timeout 1800 "$A" call --dir "$R/alice" --to "${ID[$1]}" --json "$2" 2>&1 | grep '^{' | tail -1); [ -n "$o" ] && break; sleep 5; done; echo "${o:-{\"error\":\"no reply\"\}}"; }
alice_stop() { [ -n "$alice_pid" ] && taskkill //F //PID "$alice_pid" > /dev/null 2>&1; alice_pid=""; sleep 1; }
alice_serve() {
  alice_stop; rm -f "$R/alice/addr.json"
  KOPANO_RELAY_URL="$ALICE_RELAY" "$P" serve --dir "$R/alice" > "$R/alice-serve.log" 2>&1 &
  until [ -s "$R/alice/addr.json" ]; do sleep 0.3; done
  alice_pid=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter \"Name='peer.exe'\" | Where-Object { \$_.CommandLine -like '*serve*e15*alice*' } | Select-Object -First 1).ProcessId" | tr -d '\r')
}
addr_json() { python -c "import json,sys;print(json.dumps(open(sys.argv[1]).read()))" "$1"; }
pull_from_alice() { call "$1" "{\"op\":\"run\",\"args\":[\"pull\",\"--from\",\"alice-addr.json\",\"--closure\"],\"files\":{\"alice-addr.json\":$(addr_json "$R/alice/addr.json")}}"; }
laptop_addr() { call "$1" '{"op":"serve-addr"}' | python -c "import json,sys;d=json.loads(sys.stdin.read());open(sys.argv[1],'w').write(d['reply']['result']['addr'])" "$R/$1-addr.json"; }
alice_pull() { alice_stop; "$P" pull --dir "$R/alice" --from "$1" --closure 2>&1 | grep '^{' | tail -1; }

# wait until both laptops run v5 (they accept "render")
for l in home sosh; do
  until call $l '{"op":"run","args":["render","--key","x","--out","x"],"save_as":"probe.jpg"}' | grep -qE 'no photo|not on this device|"code"'; do sleep 20; done
  say "v5 online" $l '"ok"'
done
H=$(alice_stop; "$P" hash-of --dir "$R/alice" --key $KEY | tr -d '\r')
alice_serve
for l in home sosh; do say "rejoin" $l "$(pull_from_alice $l)"; done
say "home adds contrast" home "$(call home "{\"op\":\"run\",\"args\":[\"edit\",\"--key\",\"$KEY\",\"--field\",\"develop.contrast\",\"--value\",\"25\"]}")"
say "Soshanguve makes it warmer" sosh "$(call sosh "{\"op\":\"run\",\"args\":[\"edit\",\"--key\",\"$KEY\",\"--field\",\"develop.temperature\",\"--value\",\"6800\"]}")"
laptop_addr home; laptop_addr sosh
say "owner <- home" alice "$(alice_pull "$R/home-addr.json")"
say "owner <- Soshanguve" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
for l in home sosh; do say "laptop <- owner" $l "$(pull_from_alice $l)"; done
for l in home sosh; do
  say "fetch original if missing" $l "$(call $l "{\"op\":\"run\",\"args\":[\"fetch-media\",\"--from\",\"alice-addr.json\",\"--hash\",\"$H\"],\"files\":{\"alice-addr.json\":$(addr_json "$R/alice/addr.json")},\"timeout_s\":1700}")"
done
alice_stop
for l in home sosh; do
  name=final-$([ $l = home ] && echo home-laptop || echo soshanguve).jpg
  say "render final image" $l "$(call $l "{\"op\":\"run\",\"args\":[\"render\",\"--key\",\"$KEY\",\"--out\",\"x\"],\"save_as\":\"$name\"}")"
  call $l "{\"op\":\"say\",\"text\":\"Your final image is ready: kopano-test\\\\received\\\\$name . Share it on WhatsApp and compare.\"}" > /dev/null
done
say "render final image" alice "$("$P" render --dir "$R/alice" --key $KEY --out "$R/final-owner-pc.jpg" 2>&1 | grep '^{')"
