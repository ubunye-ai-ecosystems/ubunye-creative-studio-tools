#!/usr/bin/env bash
# E14: Alice on this PC, Bob in a Cape Town sandbox. Compare our own relay with n0's public relays.
# Usage: e14_relay_run.sh <peer.exe> <root> <sandbox id> <our relay url> <runs per mode>
set -u
P="$1"; R="$2"; SID="$3"; OWN="$4"; N="${5:-3}"
SX=/c/tmp/sx.sh
serve_pid=""
stop_alice() { [ -n "$serve_pid" ] && taskkill //F //PID "$serve_pid" > /dev/null 2>&1; serve_pid=""; sleep 1; }
start_alice() {  # $1 = relay url or "", $2 = --relay-only or ""
  rm -f "$R/alice/addr.json"
  if [ -n "$1" ]; then export KOPANO_RELAY_URL="$1"; else unset KOPANO_RELAY_URL; fi
  "$P" serve --dir "$R/alice" $2 > "$R/alice-serve.log" 2>&1 &
  until [ -s "$R/alice/addr.json" ]; do sleep 0.3; done
  serve_pid=$(tasklist //FI "IMAGENAME eq peer.exe" //FO CSV //NH | tail -1 | cut -d'"' -f4)
}
bob_pull() {  # $1 = relay url or "", $2 = --relay-only or ""
  local a; a=$(base64 -w0 "$R/alice/addr.json")
  local env=""; [ -n "$1" ] && env="KOPANO_RELAY_URL=$1"
  timeout 200 "$SX" "$SID" 150000 "echo $a | base64 -d > /tmp/bob/alice-addr.json; cd /tmp && $env /tmp/kt/release/peer pull --dir /tmp/bob --from /tmp/bob/alice-addr.json --closure $2 2>&1 | grep '^{' | tail -1" | grep '^{'
}
i=0
run_mode() {  # name, relay url, relay-only flag
  for k in $(seq 1 "$N"); do
    i=$((i + 1))
    stop_alice
    unset KOPANO_RELAY_URL
    "$P" edit --dir "$R/alice" --key "IMG_$(printf %07d $i).ARW" --field develop.exposure --value "0.$i" > /dev/null
    start_alice "$2" "$3"
    out=$(bob_pull "$2" "$3")
    echo "{\"mode\":\"$1\",\"run\":$k,\"pull\":${out:-null}}"
  done
}
start_alice "$OWN" ""
echo "{\"mode\":\"join (first sync of 1,000 photos, our relay)\",\"pull\":$(bob_pull "$OWN" "")}"
run_mode "our relay, relay only" "$OWN" "--relay-only"
run_mode "n0 public relay, relay only" "" "--relay-only"
run_mode "our relay, direct allowed" "$OWN" ""
stop_alice
