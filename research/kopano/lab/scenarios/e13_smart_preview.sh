#!/usr/bin/env bash
# E13 step 2: does Light's own smart preview (layer one) develop like the original RAW?
# Usage: e13_smart_preview.sh <lightcraft-cli> <raw file> <scratch dir>
# Works on a COPY of the raw; the copy is hidden to play "original not arrived yet".
set -euo pipefail
L="$1"; SRC="$2"; W="$3"
name=$(basename "$SRC"); stem="${name%.*}"
D="$W/$stem"; rm -rf "$D"; mkdir -p "$D/orig" "$D/out"
cp "$SRC" "$D/orig/$name"
LIB="$D/lib"

run() { "$L" run --library "$LIB" "$@" 2>&1 | grep -v '^lightcraft-cli: ' ; }

# sender: import and build the smart preview
run --import "$D/orig/$name" library.selectAll library.smartPreviews > "$D/build.jsonl"
sp=$(find "$LIB" -name '*.lcsp' | head -1)
echo "{\"file\":\"$name\",\"original_bytes\":$(stat -c %s "$SRC"),\"smart_preview_bytes\":$(stat -c %s "$sp")}"

render() {  # $1 = label, $2 = exposure
  run library.selectAll develop.set control=light.exposure value="$2" \
      photo.smartPreview app.export path="$D/out/$1.ev$2.png" longEdge=2560 conflict=overwrite > "$D/out/$1.ev$2.jsonl"
  grep -o '"originalOnline":[a-z]*' "$D/out/$1.ev$2.jsonl" | head -1
}
for ev in -2 0 2; do echo "original ev$ev $(render original "$ev")"; done
mv "$D/orig/$name" "$D/orig/$name.away"           # receiver has layer one only
for ev in -2 0 2; do echo "smart ev$ev $(render smart "$ev")"; done
mv "$D/orig/$name.away" "$D/orig/$name"           # layer two arrives
echo "{\"done\":\"$stem\"}"
