#!/usr/bin/env bash
# E16 BMW round: brother makes the BMW black and white, home laptop makes it warm, this PC makes it
# cold (a deliberate clash), all at the same time; sync; each device renders its own final image.
# Usage: e16_bmw.sh <bin dir> <root> <home id> <sosh id> [alice relay url]
set -u
BIN="$1"; R="$2"; P="$BIN/peer.exe"; A="$BIN/agent.exe"
declare -A ID=( [home]="$3" [sosh]="$4" )
ALICE_RELAY="${5:-https://use1-1.relay.n0.iroh.link./}"
KEY=DSC02384.ARW
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
edit() { call "$1" "{\"op\":\"run\",\"args\":[\"edit\",\"--key\",\"$KEY\",\"--field\",\"$2\",\"--value\",\"$3\"]}"; }

H=$("$P" hash-of --dir "$R/alice" --key $KEY | tr -d '\r')
alice_serve
# each laptop: rejoin, then prove it runs v6 (v6 can add the new saturation setting; v5 cannot)
for l in home sosh; do
  until o=$(pull_from_alice $l) && echo "$o" | grep -q '"code":0' && call $l '{"op":"run","args":["edit","--key","IMG_0000001.ARW","--field","develop.saturation","--value","100"]}' | grep -q '"code":0'; do sleep 20; done
  say "v6 online and joined" $l "$o"
done
# the same moment: this PC cold, home warm, Soshanguve black and white
alice_stop
say "desktop makes it cold" alice "$("$P" edit --dir "$R/alice" --key $KEY --field develop.temperature --value 3500 | grep '^{')"
say "home makes it warm" home "$(edit home develop.temperature 7500)"
say "brother makes it black and white" sosh "$(edit sosh develop.saturation 0)"
laptop_addr home; laptop_addr sosh
say "desktop <- home (clash expected)" alice "$(alice_pull "$R/home-addr.json")"
say "desktop <- Soshanguve" alice "$(alice_pull "$R/sosh-addr.json")"
alice_serve
for l in home sosh; do say "laptop <- desktop" $l "$(pull_from_alice $l)"; done
for l in home sosh; do
  say "fetch the BMW original (49 MB)" $l "$(call $l "{\"op\":\"run\",\"args\":[\"fetch-media\",\"--from\",\"alice-addr.json\",\"--hash\",\"$H\"],\"files\":{\"alice-addr.json\":$(addr_json "$R/alice/addr.json")},\"timeout_s\":1700}")"
done
alice_stop
for l in home sosh; do
  name=final-bmw-$([ $l = home ] && echo home-laptop || echo soshanguve).jpg
  say "render the BMW" $l "$(call $l "{\"op\":\"run\",\"args\":[\"render\",\"--key\",\"$KEY\"],\"save_as\":\"$name\"}")"
  call $l "{\"op\":\"say\",\"text\":\"BMW final image ready: open the kopano-test folder, then received, then $name\"}" > /dev/null
done
say "render the BMW" alice "$("$P" render --dir "$R/alice" --key $KEY --out "$R/final-bmw-desktop.jpg" 2>&1 | grep '^{')"
say "desktop status" alice "$("$P" status --dir "$R/alice" | grep -E "CLASH|$KEY" | python -c "import sys,json;print(json.dumps({'output':sys.stdin.read()[:600]}))")"
