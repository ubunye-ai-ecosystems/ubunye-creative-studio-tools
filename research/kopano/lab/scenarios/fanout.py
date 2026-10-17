"""Experiment E8: team fan-out. One owner (Alice) serves; N teammates each run their own
process with their own device key and pull at the same moment after Alice saves one edit.
Measures the time until every teammate has verified and applied it, for N = 2, 5, 10, 25.
All peers run on this one PC, so this measures protocol + CPU + disk contention, not WAN.

Usage: python fanout.py <peer.exe> <root> [n ...]
"""
import json, os, subprocess, sys, time

P, ROOT = sys.argv[1], sys.argv[2]
SIZES = [int(x) for x in sys.argv[3:]] or [2, 5, 10, 25]


def wait_addr(d):
    for _ in range(600):
        if os.path.exists(d + "/addr.json") and os.path.getsize(d + "/addr.json") > 0:
            return
        time.sleep(0.1)
    raise SystemExit("no addr")


def serve(d):
    if os.path.exists(d + "/addr.json"):
        os.remove(d + "/addr.json")
    s = subprocess.Popen([P, "serve", "--dir", d], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    wait_addr(d)
    return s


for n in SIZES:
    root = f"{ROOT}/fanout{n}"
    subprocess.run(["rm", "-rf", root])
    subprocess.run([P, "setup", "--root", root, "--synthetic", "200", "--extra-peers", str(n)], capture_output=True, check=True)
    members = [f"{root}/m{i}" for i in range(n)]
    # Everyone joins first (initial sync), so the measured round is one small edit.
    s = serve(f"{root}/alice")
    for m in members:
        subprocess.run([P, "pull", "--dir", m, "--from", f"{root}/alice/addr.json"], capture_output=True, check=True)
    s.kill(); s.wait()
    subprocess.run([P, "edit", "--dir", f"{root}/alice", "--key", "IMG_0000007.ARW", "--field", "develop.exposure", "--value", "0.5"], capture_output=True, check=True)
    s = serve(f"{root}/alice")
    t0 = time.time()
    procs = [subprocess.Popen([P, "pull", "--dir", m, "--from", f"{root}/alice/addr.json"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for m in members]
    results = []
    for p in procs:
        out, err = p.communicate()
        results.append((time.time() - t0, out.strip() or err.strip()[-200:]))
    s.kill(); s.wait()
    ok = sum(1 for _, o in results if '"fast-forward, 1 item(s) applied"' in o)
    last = max(t for t, _ in results)
    transfer = [json.loads(o)["transfer_ms"] for _, o in results if o.startswith("{")]
    print(json.dumps({"peers": n, "applied": ok, "all_done_s": round(last, 2), "transfer_ms_max": max(transfer or [0]), "transfer_ms_median": sorted(transfer)[len(transfer) // 2] if transfer else None}), flush=True)
