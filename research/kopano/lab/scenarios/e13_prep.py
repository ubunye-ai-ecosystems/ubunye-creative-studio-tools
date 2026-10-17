"""Prepare the E13 send bundle: layer two = RAW packs + manifest of original SHA-256s (CC0 samples only)."""
import sys, os, json, hashlib, time
import rawpack as R
out = sys.argv[1]; os.makedirs(out, exist_ok=True)
man = []
for p in sys.argv[2:]:
    t = time.perf_counter(); c, rep = R.pack(p); dt = time.perf_counter() - t
    n = os.path.basename(p)
    open(os.path.join(out, n + '.krp'), 'wb').write(c)
    man.append(dict(name=n, sha256=rep['sha256'], size=rep['size'], packed=len(c), method=rep['chosen'], pack_s=round(dt, 1)))
    print(json.dumps(man[-1]), flush=True)
json.dump(man, open(os.path.join(out, 'manifest.json'), 'w'), indent=1)
