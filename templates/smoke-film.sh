#!/usr/bin/env bash
# Headless smoke test for ubunye-creative-film:
#   import still → sequence from clip → apply Invert effect → save .fcproj → reopen → render + export ProRes → verify.
# Usage: smoke-film.sh <path-to-filmcraft-cli> [workdir]
set -euo pipefail

CLI=${1:?path to filmcraft-cli}
WORK=${2:-$(mktemp -d)}
mkdir -p "$WORK" && cd "$WORK"
rm -f *.fcproj r0.png r1.png out.mov

python3 -I - <<'PY'
import struct, zlib
w, h = 64, 48
rows = b"".join(b"\x00" + b"".join(bytes([(x * 4) % 256, (y * 5) % 256, 128]) for x in range(w)) for y in range(h))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
open("in.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY

j() { python3 -I -c "import json,sys; print(eval(sys.argv[1], {}, {'d': json.load(sys.stdin)}))" "$1"; }

ITEM=$("$CLI" --compact --save-as p0.fcproj import in.png | j 'd["items"][0]')
CLIP=$("$CLI" --compact --project p0.fcproj --save exec file.newSequenceFromClip "items=[$ITEM]" | j 'd["clips"][0]')
"$CLI" --compact --project p0.fcproj render --seconds 1 --out r0.png >/dev/null              # control frame
"$CLI" --compact --project p0.fcproj --save-as p1.fcproj exec effects.apply "clips=[$CLIP]" effect=invert >/dev/null

# Reopen in a fresh process: the effect must have been saved.
"$CLI" --compact --project p1.fcproj inspect sequence > seq.json
grep -q '"invert"' p1.fcproj || { echo "FAIL: invert effect not saved in project"; exit 1; }
"$CLI" --compact --project p1.fcproj render --seconds 1 --out r1.png >/dev/null
cmp -s r0.png r1.png && { echo "FAIL: effect had no visible result"; exit 1; }
"$CLI" --compact --project p1.fcproj export out.mov --format prores > export.json
"$CLI" --compact probe out.mov > probe.json

python3 -I - <<'PY'
import json, sys
e = json.load(open("export.json")); p = json.load(open("probe.json"))
frames = e["result"]["frames"]; codec = p["video"]["codec"]
if frames <= 0 or "ProRes" not in codec:
    sys.exit(f"FAIL: export frames={frames} codec={codec}")
print(f"PASS: import → effect → save .fcproj → reopen → export ({frames} frames, {codec})")
PY
