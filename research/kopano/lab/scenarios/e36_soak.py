"""E36: a long run. The owner's daemon edits every 1-4 s for MINUTES; three teammates follow over a
flaky slow link (relay behind the chaos proxy: 1 Mbit/s, 150 ms each way, connection cut every
20 s). Pass: after the edits stop, every teammate holds exactly the owner's values; memory flat.
Usage: python e36_soak.py <bin dir> <iroh-relay.exe> <work dir> <minutes>"""
import json, os, random, shutil, socket, subprocess, sys, time
import psutil

BIN, RELAY, W, MIN = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
P, C = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'chaos.exe')
random.seed(36)
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '5000', '--extra-peers', '2'], capture_output=True, check=True)
relay = subprocess.Popen([RELAY, '--dev'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(2)
chaos = subprocess.Popen([C, '--listen', '127.0.0.1:4000', '--to', '[::1]:3340', '--kbps', '1000', '--delay-ms', '150', '--drop-every-s', '20'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
env = dict(os.environ, KOPANO_RELAY_URL='http://127.0.0.1:4000')
procs = {'alice': subprocess.Popen([P, 'daemon', '--dir', f'{W}/alice', '--port', '7400', '--relay-only'], env=env, stdout=subprocess.DEVNULL, stderr=open(f'{W}/alice.err', 'w'))}
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)
for i, n in enumerate(['bob', 'm0', 'm1']):
    procs[n] = subprocess.Popen([P, 'daemon', '--dir', f'{W}/{n}', '--port', str(7401 + i), '--follow', f'{W}/alice/addr.json', '--relay-only'], env=env, stdout=open(f'{W}/{n}.out', 'w'), stderr=open(f'{W}/{n}.err', 'w'))
ports = {'alice': 7400, 'bob': 7401, 'm0': 7402, 'm1': 7403}

def call(name, req):
    with socket.create_connection(('127.0.0.1', ports[name]), timeout=10) as c:
        f = c.makefile('rw')
        f.write(json.dumps(req) + chr(10))
        f.flush()
        return json.loads(f.readline())

time.sleep(20)
mem = {n: [] for n in procs}
last = {}
edits, t0, next_mem = 0, time.time(), 0
while time.time() - t0 < MIN * 60:
    k = f'IMG_{random.randrange(5000):07d}.ARW'
    v = round(random.uniform(-2, 2), 3)
    r = call('alice', {'op': 'set', 'key': k, 'field': 'develop.exposure', 'value': v})
    if r.get('ok'):
        last[k] = v
        edits += 1
    if time.time() > next_mem:
        for n, p in procs.items():
            mem[n].append(round(psutil.Process(p.pid).memory_info().rss / 1e6, 1))
        next_mem = time.time() + 60
    time.sleep(random.uniform(1, 4))
# let the last pushes land (a cut may be in progress), then compare every edited photo
deadline = time.time() + 180
mismatch = {}
while time.time() < deadline:
    mismatch = {n: sum(1 for k, v in last.items() if call(n, {'op': 'get', 'key': k, 'field': 'develop.exposure'}).get('result') != v) for n in ('bob', 'm0', 'm1')}
    if not any(mismatch.values()):
        break
    time.sleep(15)
reconnects = {n: open(f'{W}/{n}.err').read().count('interrupted') for n in ('bob', 'm0', 'm1')}
for p in list(procs.values()) + [chaos, relay]:
    p.kill()
print(json.dumps({'minutes': MIN, 'edits': edits, 'photos_edited': len(last), 'mismatched_after_settle': mismatch, 'settled_s': round(180 - (deadline - time.time())), 'reconnects': reconnects,
                  'rss_mb_first_last_max': {n: (m[0], m[-1], max(m)) for n, m in mem.items() if m}}))
