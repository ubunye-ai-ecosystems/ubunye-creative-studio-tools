#!/usr/bin/env bash
# E18: Kopano over bad networks. Local iroh-relay behind the chaos proxy; two peers on this PC,
# relay-only so every byte crosses the "network". One JSON line per measurement.
# Usage: e18_net.sh <bin dir> <iroh-relay.exe> <work dir> <media file>
set -u
BIN="$1"; RELAY_EXE="$2"; W="$3"; MEDIA="$4"
P="$BIN/peer.exe"; C="$BIN/chaos.exe"
export KOPANO_RELAY_URL="http://127.0.0.1:4000"
mkdir -p "$W"
"$RELAY_EXE" --dev > "$W/relay.log" 2>&1 &
RELAY_PID=$!
sleep 2

kill_win() { taskkill //F //IM "$1" > /dev/null 2>&1; }
serve_alice() {
  rm -f "$R/alice/addr.json"
  "$P" serve --dir "$R/alice" --relay-only > "$R/serve.log" 2>&1 &
  local t=0; until [ -s "$R/alice/addr.json" ] || [ $t -gt 600 ]; do sleep 0.5; t=$((t+1)); done
}
stop_alice() { powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='peer.exe'\" | Where-Object { \$_.CommandLine -like '*serve*e18*alice*' } | ForEach-Object { Stop-Process -Id \$_.ProcessId -Force }" > /dev/null 2>&1; sleep 1; }
emit() { echo "{\"profile\":\"$1\",\"step\":\"$2\",\"wall_s\":$3,\"out\":${4:-null}}"; }
now() { date +%s.%N; }
el() { python -c "print(round($2-$1,2))"; }

# name kbps one-way-delay-ms drop-every-s
PROFILES="${PROFILES_OVERRIDE:-nobox:0:0:0 fibre:50000:10:0 lte:10000:30:0 3g:1000:150:0 edge:100:300:0 flaky:1000:150:20}"
for prof in $PROFILES; do
  IFS=: read name kbps delay drop <<< "$prof"
  R="$W/$name"; rm -rf "$R"
  if [ "$name" = nobox ]; then export KOPANO_RELAY_URL="http://localhost:3340"; else export KOPANO_RELAY_URL="http://127.0.0.1:4000"
  "$C" --listen 127.0.0.1:4000 --to "[::1]:3340" --kbps $kbps --delay-ms $delay --drop-every-s $drop > "$W/chaos-$name.log" 2>&1 & fi
  sleep 1
  "$P" setup --root "$R" --video "$MEDIA" --synthetic 1000 > /dev/null 2>&1
  serve_alice
  s=$(now); o=$(timeout 1800 "$P" pull --dir "$R/bob" --from "$R/alice/addr.json" --closure ${PULL_EXTRA:-} --relay-only 2>&1 | grep '^{' | tail -1); emit $name "join 1,000 photos" $(el $s $(now)) "${o:-null}"
  stop_alice; "$P" edit --dir "$R/alice" --key IMG_0000007.ARW --field develop.exposure --value 0.4 > /dev/null; serve_alice
  s=$(now); o=$(timeout 900 "$P" pull --dir "$R/bob" --from "$R/alice/addr.json" --closure ${PULL_EXTRA:-} --relay-only 2>&1 | grep '^{' | tail -1); emit $name "one small edit" $(el $s $(now)) "${o:-null}"
  if [ "${SKIP_MEDIA:-0}" = 1 ]; then stop_alice; kill_win chaos.exe; continue; fi
  H=$(stop_alice; "$P" hash-of --dir "$R/alice" --key "$(basename "$MEDIA")" | tr -d '\r'); serve_alice
  s=$(now); tries=0; done_=false
  while [ $tries -lt 40 ]; do
    tries=$((tries+1))
    o=$(timeout 1800 "$P" fetch-media --dir "$R/bob" --from "$R/alice/addr.json" --hash "$H" --relay-only 2>&1 | grep '^{' | tail -1)
    echo "$o" | grep -qE '"complete":true|already_complete' && { done_=true; break; }
    sleep 2
  done
  emit $name "4 MB camera JPEG (resumes after drops)" $(el $s $(now)) "{\"attempts\":$tries,\"complete\":$done_,\"last\":${o:-null}}"
  stop_alice
  kill_win chaos.exe
done
kill_win iroh-relay.exe
