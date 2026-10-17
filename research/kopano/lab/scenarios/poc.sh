#!/usr/bin/env bash
# Kopano proof of concept: two (three) independently identified peers, as separate OS processes
# on one Windows PC. Every step prints one JSON line; the whole run is saved as evidence.
#
# Usage: poc.sh <peer.exe> <root dir> <photo1> <photo2> <photo3> <video>
# Media files stay on this machine. Only Kopano metadata ever crosses a public relay (step 6),
# and QUIC encrypts it end to end; the relay cannot read it.
set -u
PEER="$1"; ROOT="$2"; P1="$3"; P2="$4"; P3="$5"; VID="$6"
rm -rf "$ROOT"; mkdir -p "$ROOT"
step() { echo; echo "### $*"; }
wait_addr() { for i in $(seq 1 200); do [ -s "$1/addr.json" ] && return 0; sleep 0.2; done; echo "no addr"; return 1; }
serve() { rm -f "$ROOT/$1/addr.json"; "$PEER" serve --dir "$ROOT/$1" $2 > "$ROOT/$1.serve.log" 2>&1 & echo $!; }
stop() { kill "$1" 2>/dev/null; for i in $(seq 1 100); do kill -0 "$1" 2>/dev/null || break; sleep 0.1; done; sleep 0.3; }

step "1. Setup: provider, devices for Alice, Bob, Carol; project owned by Alice, Bob is editor of the develop lane; Carol is not a member"
"$PEER" setup --root "$ROOT" --photos "$P1" "$P2" "$P3" --video "$VID"
KEY=$(basename "$P1")
VKEY=$(basename "$VID")

step "2. Alice comes online"
A=$(serve alice ""); wait_addr "$ROOT/alice"; cat "$ROOT/alice.serve.log"

step "3. Bob joins: pulls history and edit state (no media)"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"

step "4. Bob already has the photo originals from the card: nothing to transfer"
"$PEER" fetch-media --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" --hash "$("$PEER" hash-of --dir "$ROOT/bob" --key "$KEY")"

step "5. Alice makes a small edit; Bob pulls it (bytes for one exposure change)"
stop $A   # one process owns a device's database; in a product the Kopano service does the edit
"$PEER" edit --dir "$ROOT/alice" --key "$KEY" --field develop.exposure --value 0.35 --message "Lift exposure"
A=$(serve alice ""); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"

step "6. Same over the relay only (no direct path allowed): worst-case NAT"
stop $A
"$PEER" edit --dir "$ROOT/alice" --key "$KEY" --field develop.temperature --value 5900 --message "Warmer"
A=$(serve alice "--relay-only"); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" --relay-only
stop $A; A=$(serve alice ""); wait_addr "$ROOT/alice"

step "7. Carol has a valid account but is not a member: refused"
"$PEER" pull --dir "$ROOT/carol" --from "$ROOT/alice/addr.json" 2>&1 | tail -1
"$PEER" fetch-media --dir "$ROOT/carol" --from "$ROOT/alice/addr.json" --hash "$("$PEER" hash-of --dir "$ROOT/bob" --key "$VKEY")" 2>&1 | tail -1

step "8. Bob fetches the video original; the transfer is killed part way; then resumed"
VH=$("$PEER" hash-of --dir "$ROOT/bob" --key "$VKEY")
"$PEER" fetch-media --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" --hash "$VH" & F=$!
sleep 1.5; kill $F 2>/dev/null; wait $F 2>/dev/null; echo "(killed the transfer after 1.5 s)"
"$PEER" fetch-media --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" --hash "$VH"
"$PEER" fetch-media --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" --hash "$VH"

step "9. Concurrent edits, different settings: Alice exposure, Bob contrast, offline from each other"
stop $A
"$PEER" edit --dir "$ROOT/alice" --key "$KEY" --field develop.exposure --value 0.6 --message "Brighter"
"$PEER" edit --dir "$ROOT/bob" --key "$KEY" --field develop.contrast --value 25 --message "More contrast"
A=$(serve alice ""); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"

step "10. Concurrent edits, same setting: Alice exposure 0.9, Bob exposure -0.4"
stop $A
"$PEER" edit --dir "$ROOT/alice" --key "$KEY" --field develop.exposure --value 0.9 --message "Much brighter"
"$PEER" edit --dir "$ROOT/bob" --key "$KEY" --field develop.exposure --value -0.4 --message "Darker, moodier"
A=$(serve alice ""); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"
"$PEER" status --dir "$ROOT/bob"

step "11. Alice pulls from Bob: gets the merge and the open clash record"
stop $A
B=$(serve bob ""); wait_addr "$ROOT/bob"
"$PEER" pull --dir "$ROOT/alice" --from "$ROOT/bob/addr.json"
"$PEER" status --dir "$ROOT/alice"

step "12. Integrity: a byte of Bob's newest version rots on disk; detect, repair, re-fetch"
stop $B
"$PEER" corrupt --dir "$ROOT/bob"
"$PEER" verify --dir "$ROOT/bob"
"$PEER" repair --dir "$ROOT/bob"
A=$(serve alice ""); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"
"$PEER" verify --dir "$ROOT/bob"

step "13. Alice goes offline mid-session; Bob's pull fails cleanly; Alice returns; pull succeeds"
stop $A
timeout 40 "$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json" 2>&1 | tail -1; echo "(exit $?)"
A=$(serve alice ""); wait_addr "$ROOT/alice"
"$PEER" pull --dir "$ROOT/bob" --from "$ROOT/alice/addr.json"
stop $A
echo; echo "### done"
