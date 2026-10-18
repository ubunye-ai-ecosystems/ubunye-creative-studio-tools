"""E24: one owner's daemon pushes each edit to N teammates' daemons; time until all applied.
Usage: python e24_fanout_push.py <peer.exe> <work dir> <N>"""
import json, os, shutil, socket, subprocess, sys, time

P, W, N = sys.argv[1], sys.argv[2], int(sys.argv[3])
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '1000', '--extra-peers', str(N - 1)], capture_output=True)
names = ['bob'] + [f'm{i}' for i in range(N - 1)]
a = subprocess.Popen([P, 'daemon', '--dir', f'{W}/alice', '--port', '7200'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)
procs = []
for i, n in enumerate(names):
    procs.append(subprocess.Popen([P, 'daemon', '--dir', f'{W}/{n}', '--port', str(7201 + i), '--follow', f'{W}/alice/addr.json'], stdout=open(f'{W}/{n}.out', 'w'), stderr=subprocess.STDOUT))

def lines(n):
    out = []
    try:
        for l in open(f'{W}/{n}.out'):
            if l.startswith('{') and l.endswith(chr(10)):
                try:
                    out.append(json.loads(l))
                except ValueError:
                    pass
    except OSError:
        pass
    return out

t0 = time.time()
while sum(1 for n in names if lines(n)) < N and time.time() - t0 < 300:
    time.sleep(0.5)
joined = sum(1 for n in names if lines(n))
join_s = round(time.time() - t0, 1)
s = socket.create_connection(('127.0.0.1', 7200))
f = s.makefile('rw')
rows = []
for k in range(5):
    before = {n: len(lines(n)) for n in names}
    f.write(json.dumps({'op': 'set', 'key': f'IMG_{k:07d}.ARW', 'field': 'develop.exposure', 'value': 0.1 * (k + 1)}) + '\n')
    f.flush()
    t_edit = json.loads(f.readline())['result']['ms']
    t1 = time.time()
    while time.time() - t1 < 60:
        if all(len(lines(n)) > before[n] for n in names):
            break
        time.sleep(0.05)
    lat = [lines(n)[-1]['now_ms'] - t_edit for n in names if len(lines(n)) > before[n]]
    rows.append({'reached': len(lat), 'median_ms': sorted(lat)[len(lat) // 2] if lat else None, 'slowest_ms': max(lat) if lat else None})
    time.sleep(1)
for p in procs + [a]:
    p.kill()
print(json.dumps({'teammates': N, 'joined': joined, 'all_joined_s': join_s, 'edits': rows}))
