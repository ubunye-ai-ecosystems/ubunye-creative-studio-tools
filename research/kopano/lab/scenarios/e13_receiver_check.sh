set -u
LC=/tmp/light/target/release/lightcraft-cli
PY=/tmp/venv/bin/python
OUT=/tmp/out; rm -rf $OUT; mkdir -p $OUT/smart $OUT/original /tmp/orig
LIB=/tmp/recv/layer1/lib; rm -f $LIB/catalog.lock
render_all() {  # $1 library $2 outdir
  ids=$($LC run --library "$1" catalog.query 2>/dev/null | $PY -c "
import sys, json
for l in sys.stdin:
    for p in json.loads(l)['result']['photos']: print(f\"{p['id']}:{p['fileName']}\")")
  for item in $ids; do id=${item%%:*}; name=${item#*:}
    for ev in -2 0 2; do
      $LC run --library "$1" library.select "ids=[$id]" active=$id develop.set control=light.exposure value=$ev photo.smartPreview \
        app.export "path=$2/$name.ev$ev.png" "ids=[$id]" longEdge=2560 conflict=overwrite 2>/dev/null | $PY -c "
import sys, json
on = None
for l in sys.stdin:
    d = json.loads(l)
    if d['command'] == 'photo.smartPreview': on = d['result']['originalOnline']
    if d['command'] == 'app.export': print('$name ev$ev', 'ok' if d['ok'] else d.get('error'), 'originalOnline=' + str(on))"
    done
  done
}
echo "== 1. layer one only: Light on Linux renders from smart previews"
S=$(date +%s); render_all $LIB $OUT/smart; echo "secs $(( $(date +%s) - S ))"
echo "== 2. layer two: unpack and check fingerprints"
cd /vercel/ubunye-creative-studio-tools/research/kopano/lab/scenarios
KOPANO_LJ92=/tmp/lj92/target/release/lj92 $PY - <<'PYEOF'
import json, hashlib, time, rawpack as R
man = json.load(open('/tmp/recv/layer2/manifest.json'))
for m in man:
    t = time.perf_counter()
    data = R.unpack(open(f"/tmp/recv/layer2/{m['name']}.krp", 'rb').read())
    dt = time.perf_counter() - t
    h = hashlib.sha256(data).hexdigest()
    open(f"/tmp/orig/{m['name']}", 'wb').write(data)
    print(json.dumps(dict(name=m['name'], method=m['method'], bytes=len(data), unpack_s=round(dt, 2), identical=h == m['sha256'])))
PYEOF
echo "== 3. originals rebuilt on Linux, rendered by Light on Linux"
for f in /tmp/orig/*; do n=$(basename $f); for ev in -2 0 2; do $LC render $f -o $OUT/original/$n.ev$ev.png --size 2560 --set light.exposure=$ev 2>&1 | tail -1; done; done
ls $OUT/smart | wc -l; ls $OUT/original | wc -l
