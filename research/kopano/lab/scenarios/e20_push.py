"""E20: edit-to-teammate latency with always-on daemons and push, per network profile.
Usage: python e20_push.py <bin dir> <iroh-relay.exe> <work dir>"""
import json, os, shutil, socket, subprocess, sys, time

BIN, RELAY, W = sys.argv[1:4]
P, C = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'chaos.exe')
PROFILES = [('lte', 10000, 30, 0), ('3g', 1000, 150, 0), ('edge', 100, 300, 0), ('flaky', 1000, 150, 20)]
os.makedirs(W, exist_ok=True)
relay = subprocess.Popen([RELAY, '--dev'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(2)
env = dict(os.environ, KOPANO_RELAY_URL='http://127.0.0.1:4000')
results = []
for name, kbps, delay, drop in PROFILES:
    R = os.path.join(W, name)
    shutil.rmtree(R, ignore_errors=True)
    chaos = subprocess.Popen([C, '--listen', '127.0.0.1:4000', '--to', '[::1]:3340', '--kbps', str(kbps), '--delay-ms', str(delay), '--drop-every-s', str(drop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    subprocess.run([P, 'setup', '--root', R, '--synthetic', '1000'], capture_output=True)
    a = subprocess.Popen([P, 'daemon', '--dir', f'{R}/alice', '--port', '7101', '--relay-only'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not os.path.exists(f'{R}/alice/addr.json'):
        time.sleep(0.3)
    blog = open(f'{R}/bob.out', 'w')
    b = subprocess.Popen([P, 'daemon', '--dir', f'{R}/bob', '--port', '7102', '--follow', f'{R}/alice/addr.json', '--relay-only'], env=env, stdout=blog, stderr=subprocess.STDOUT)
    def synced():
        out = []
        try:
            for l in open(f'{R}/bob.out'):
                if l.startswith('{') and l.endswith(chr(10)):
                    try:
                        out.append(json.loads(l))
                    except ValueError:
                        pass  # a line still being written
        except OSError:
            pass
        return out
    t0 = time.time()
    while not synced() and time.time() - t0 < 300:  # Bob's first sync: the whole project
        time.sleep(0.2)
    join = synced()[0] if synced() else None
    s = socket.create_connection(('127.0.0.1', 7101))
    f = s.makefile('rw')
    lat = []
    for k in range(10):
        before = len(synced())
        f.write(json.dumps({'op': 'set', 'key': f'IMG_{k:07d}.ARW', 'field': 'develop.exposure', 'value': 0.1 * (k + 1)}) + '\n')
        f.flush()
        rep = json.loads(f.readline())
        t_edit = rep['result']['ms']
        t1 = time.time()
        while len(synced()) == before and time.time() - t1 < 120:
            time.sleep(0.02)
        if len(synced()) > before:
            ev = synced()[-1]
            lat.append(ev['now_ms'] - t_edit)
        time.sleep(1.5)
    s.close()
    for proc in (a, b, chaos):
        proc.kill()
    time.sleep(1)
    lat_sorted = sorted(lat)
    row = {'profile': name, 'join': join and {k: join[k] for k in ('bytes', 'apply_ms')}, 'edits_synced': len(lat), 'latency_ms_median': lat_sorted[len(lat) // 2] if lat else None, 'latency_ms_max': max(lat) if lat else None, 'latency_ms': lat}
    print(json.dumps(row), flush=True)
    results.append(row)
relay.kill()
