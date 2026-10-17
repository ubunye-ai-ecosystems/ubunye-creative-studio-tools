#!/usr/bin/env bash
# Render every photo of a Light library at exposure -2, 0, +2 (from whatever Light has: original or smart preview).
# Usage: e13_render_lib.sh <lightcraft-cli> <library> <out dir>
set -euo pipefail
PY3=$(command -v python3 || command -v python)
L="$1"; LIB="$2"; OUT="$3"; mkdir -p "$OUT"
ids=$("$L" run --library "$LIB" catalog.query 2>/dev/null | "$PY3" -c "
import sys, json
for l in sys.stdin:
    d = json.loads(l)
    for p in d['result']['photos']:
        print(f\"{p['id']}:{p['fileName']}\")" | tr -d '\r')
for item in $ids; do
  id="${item%%:*}"; name="${item#*:}"
  for ev in -2 0 2; do
    "$L" run --library "$LIB" library.select "ids=[$id]" active=$id develop.set control=light.exposure value=$ev \
      photo.smartPreview app.export "path=$OUT/$name.ev$ev.png" "ids=[$id]" longEdge=2560 conflict=overwrite 2>/dev/null \
      | "$PY3" -c "
import sys, json
for l in sys.stdin:
    d = json.loads(l)
    if d['command'] == 'photo.smartPreview': on = d['result']['originalOnline']
    if d['command'] == 'app.export': print('$name ev$ev', 'ok' if d['ok'] else d.get('error'), 'originalOnline=' + str(on))"
  done
done
