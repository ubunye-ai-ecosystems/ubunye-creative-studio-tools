"""E31: one big original from several teammates at once. Each teammate sits behind its own
limited link (chaos proxy in front of the relay). Runs: one teammate; two equal teammates; two
with one leaving part way; one fast and one slow (work stealing should split by speed).
Usage: python e31_swarm.py <bin dir> <iroh-relay.exe> <work dir> <file MB>"""
import json, os, shutil, subprocess, sys, time

BIN, RELAY, W, MB = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
P, C = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'chaos.exe')
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '10', '--extra-peers', '1'], capture_output=True, check=True)
with open(f'{W}/orig.bin', 'wb') as f:
    for _ in range(MB):
        f.write(os.urandom(1 << 20))
h = json.loads(subprocess.run([P, 'add-media', '--dir', f'{W}/alice', '--file', f'{W}/orig.bin'], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash']
h2 = json.loads(subprocess.run([P, 'add-media', '--dir', f'{W}/m0', '--file', f'{W}/orig.bin'], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash']
assert h == h2
# one relay server per teammate (a relay knows each device once, so sharing one would funnel
# both teammates' data through one link), plus Bob's own
relays = [subprocess.Popen([RELAY, '--dev'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)]
for port in (3341, 3342):
    cfg = f'{W}/relay{port}.toml'
    open(cfg, 'w').write(f'http_bind_addr = "[::1]:{port}"' + chr(10) + 'enable_quic_addr_discovery = false' + chr(10) + 'enable_metrics = false' + chr(10))
    relays.append(subprocess.Popen([RELAY, '--dev', '-c', cfg], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
time.sleep(2)

def start(name, port, kbps):
    chaos = subprocess.Popen([C, '--listen', f'127.0.0.1:{port}', '--to', f'[::1]:{port - 4001 + 3341}', '--kbps', str(kbps), '--delay-ms', '40', '--drop-every-s', '0'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.5)
    addr = f'{W}/{name}/addr.json'
    if os.path.exists(addr):
        os.remove(addr)
    env = dict(os.environ, KOPANO_RELAY_URL=f'http://127.0.0.1:{port}')
    p = subprocess.Popen([P, 'serve', '--dir', f'{W}/{name}', '--relay-only'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not os.path.exists(addr):
        time.sleep(0.3)
    return chaos, p, addr

def swarm(addrs, kill=None):
    d = f'{W}/bob-run'
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(f'{W}/bob', d)
    env = dict(os.environ, KOPANO_RELAY_URL='http://[::1]:3340')
    q = subprocess.Popen([P, 'fetch-swarm', '--dir', d, '--from', ','.join(addrs), '--hash', h, '--relay-only', '--stripe-mb', '4'], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if kill:
        time.sleep(kill[0])
        kill[1].kill()
    out, err = q.communicate(timeout=1800)
    ev = [json.loads(l) for l in out.splitlines() if l.startswith('{')]
    return ev[-1] if ev else {'error': err[-300:]}

rows = []
for name, links, kill_after in (
    ('one teammate', [16000], None),
    ('two equal teammates', [16000, 16000], None),
    ('two, one leaves after 8 s', [16000, 16000], 8),
    ('one fast (16 Mbit), one slow (4 Mbit)', [16000, 4000], None),
):
    procs, addrs = [], []
    for i, kbps in enumerate(links):
        c, p, a = start(['alice', 'm0'][i], 4001 + i, kbps)
        procs += [c, p]
        addrs.append(a)
    r = swarm(addrs, kill=(kill_after, procs[3]) if kill_after else None)
    row = {'run': name, 'seconds': round(r.get('ms', 0) / 1000, 1), 'mbit_s': round(r.get('mbit_s', 0), 1), 'complete': r.get('complete'), 'split_mb': [t['mb'] for t in r.get('per_teammate', [])], 'error': r.get('error', '')}
    rows.append(row)
    print(json.dumps(row), flush=True)
    for p in procs:
        p.kill()
    time.sleep(1)
for r in relays:
    r.kill()
print(json.dumps({'file_mb': MB, 'runs': rows}))
