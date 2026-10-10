#!/usr/bin/env bash
# Headless smoke test for ubunye-creative-photo:
#   import PNG → add an Invert adjustment layer → save .pcraft → reopen → export PNG → verify.
# Usage: smoke-photo.sh <path-to-photocraft-cli> [workdir]
set -euo pipefail

CLI=${1:?path to photocraft-cli}
WORK=${2:-$(mktemp -d)}
mkdir -p "$WORK" && cd "$WORK"

python3 -I - <<'PY'
import struct, zlib
w, h = 64, 48
rows = b"".join(b"\x00" + b"".join(bytes([(x * 4) % 256, (y * 5) % 256, 128]) for x in range(w)) for y in range(h))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
open("in.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY

rm -f doc.pcraft out0.png out1.png
"$CLI" convert in.png out0.png >/dev/null                                            # control
"$CLI" run in.png --cmd layer.newAdjustmentLayer.invert --out doc.pcraft >/dev/null  # edit + save project
"$CLI" info doc.pcraft --compact > info.json                                         # reopen
"$CLI" convert doc.pcraft out1.png >/dev/null                                        # export from the reopened project

python3 -I - <<'PY'
import json, struct, sys, zlib
def pixels(p):
    d = open(p, "rb").read(); i = 8; idat = b""; hdr = None
    while i < len(d):
        n, t = struct.unpack(">I4s", d[i:i + 8]); body = d[i + 8:i + 8 + n]; i += 12 + n
        if t == b"IHDR": hdr = struct.unpack(">IIBBBBB", body)
        elif t == b"IDAT": idat += body
    w, h, depth, ctype = hdr[0], hdr[1], hdr[2], hdr[3]
    assert depth == 8, depth
    bpp = {2: 3, 6: 4}[ctype]; raw = zlib.decompress(idat); stride = w * bpp; out = []; prev = bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]; line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0; b = prev[x]; c = prev[x - bpp] if x >= bpp else 0
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out.append(bytes(line)); prev = line
    return w, h, bpp, out
info = json.load(open("info.json"))
text = json.dumps(info)
if "nvert" not in text:
    sys.exit("FAIL: reopened .pcraft has no Invert adjustment layer: " + text[:300])
w, h, bpp, a = pixels("out0.png"); _, _, bpp1, b = pixels("out1.png")
x, y = 10, 7
p0 = a[y][x * bpp:x * bpp + 3]; p1 = b[y][x * bpp1:x * bpp1 + 3]
if any(abs((255 - u) - v) > 2 for u, v in zip(p0, p1)):
    sys.exit(f"FAIL: exported pixel {tuple(p1)} is not the inverse of {tuple(p0)}")
print(f"PASS: import → adjustment layer → save .pcraft → reopen → export ({tuple(p0)} → {tuple(p1)})")
PY
