"""E37: the three real machines (this desktop = Alice; home laptop; Soshanguve laptop, another
network). 1) home fetches original B from the desktop; 2) Soshanguve fetches original A from the
desktop alone (baseline); 3) Soshanguve swarm-fetches B from desktop + home at once; 4) LAN-only:
home serves with no relay and no internet lookup, the desktop syncs and fetches from it over Wi-Fi.
All with BBRv3 (KOPANO_CC=bbr).
Usage: python e37_real.py <bin dir> <alice dir> <work dir> <MB>"""
import json, os, subprocess, sys, time

BIN, ALICE, W, MB = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
P, AG = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'agent.exe')
ID = {'home': '52f7a285291c0bf09d80cfa3243724379318466ab342bff422170710e05029d8',
      'sosh': 'f2189abd27ffe41cd186316176d58634e0ecaca5efd56e74ef2677b90862651e'}
# the laptop helper only lets the relay setting change remotely (by design), so the laptops run
# their package defaults; this desktop matches them
CC = {}
os.makedirs(W, exist_ok=True)
log = open(f'{W}/e37.log', 'a')

def call(who, req, timeout=1800):
    for attempt in range(3):
        r = subprocess.run([AG, 'call', '--dir', ALICE, '--to', ID[who], '--json', json.dumps(req)], capture_output=True, text=True, timeout=timeout + 120)
        lines = [l for l in r.stdout.splitlines() if l.startswith('{')]
        if lines:
            out = json.loads(lines[-1])
            log.write(json.dumps({'who': who, 'req': req.get('op'), 'out': out}) + chr(10))
            log.flush()
            return out
        time.sleep(5)
    return {'error': r.stderr[-300:]}

def run(who, args, files=None, timeout=1800, env=None):
    out = call(who, {'op': 'run', 'args': args, 'files': files or {}, 'env': {**CC, **(env or {})}, 'timeout_s': timeout}, timeout)
    res = out.get('reply', {}).get('result', {})
    ev = [json.loads(l) for l in res.get('stdout', '').splitlines() if l.startswith('{')]
    return (ev[-1] if ev else None), res.get('stderr', '')[-300:], res.get('ms')

def addr_of(who):
    return call(who, {'op': 'serve-addr'}).get('reply', {}).get('result', {}).get('addr', '')

# originals A and B on the desktop
hashes = {}
for name in ('A', 'B'):
    f = f'{W}/orig{name}.bin'
    if not os.path.exists(f):
        with open(f, 'wb') as o:
            for _ in range(MB):
                o.write(os.urandom(1 << 20))
    hashes[name] = json.loads(subprocess.run([P, 'add-media', '--dir', ALICE, '--file', f], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash']
# the desktop serves on a different relay than the control tool uses (one identity, two programs)
addr_file = os.path.join(ALICE, 'addr.json')
if os.path.exists(addr_file):
    os.remove(addr_file)
env = dict(os.environ, KOPANO_RELAY_URL='https://use1-1.relay.n0.iroh.link./', **CC)
alice = subprocess.Popen([P, 'serve', '--dir', ALICE], env=env, stdout=subprocess.DEVNULL, stderr=open(f'{W}/alice-serve.log', 'w'))
while not os.path.exists(addr_file):
    time.sleep(0.5)
alice_addr = open(addr_file).read()
res = {'mb': MB}
for who in ('home', 'sosh'):
    call(who, {'op': 'serve', 'relay_only': False, 'env': CC})
ev, err, ms = run('home', ['fetch-media', '--from', 'a.json', '--hash', hashes['B']], {'a.json': alice_addr})
res['1_home_gets_B_from_desktop'] = {'path': ev and ev.get('path'), 's': ev and round(ev['ms'] / 1000, 1), 'mbit_s': ev and ev.get('mbit_s'), 'complete': ev and (ev.get('complete') or ev.get('already_complete')), 'err': '' if ev else err}
print(json.dumps(res), flush=True)
ev, err, ms = run('sosh', ['fetch-media', '--from', 'a.json', '--hash', hashes['A']], {'a.json': alice_addr})
res['2_sosh_gets_A_from_desktop_only'] = {'path': ev and ev.get('path'), 's': ev and round(ev['ms'] / 1000, 1), 'mbit_s': ev and ev.get('mbit_s'), 'complete': ev and (ev.get('complete') or ev.get('already_complete')), 'err': '' if ev else err}
print(json.dumps(res), flush=True)
home_addr = addr_of('home')
ev, err, ms = run('sosh', ['fetch-swarm', '--from', 'a.json,h.json', '--hash', hashes['B']], {'a.json': alice_addr, 'h.json': home_addr})
res['3_sosh_swarms_B_from_desktop_and_home'] = ev or {'err': err}
print(json.dumps(res), flush=True)
alice.kill()
# 4) LAN-only: home serves with no relay, the desktop pulls and fetches over the home Wi-Fi
# home serves normally; THIS desktop is LAN-only (no relay, no internet lookup), so it can only
# reach home over the home Wi-Fi
lan_addr = addr_of('home')
open(f'{W}/home-lan.json', 'w').write(lan_addr)
lan_env = dict(os.environ, KOPANO_LAN_ONLY='1', **CC)
r = subprocess.run([P, 'pull', '--dir', ALICE, '--from', f'{W}/home-lan.json', '--compact'], capture_output=True, text=True, env=lan_env, timeout=600)
ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
res['4_lan_only'] = {'home_addr_has_relay': 'Relay' in lan_addr, 'pull': ev[-1] if ev else r.stderr[-300:]}
import shutil
desk = f'{W}/desk-copy'
shutil.rmtree(desk, ignore_errors=True)
shutil.copytree(ALICE, desk, ignore=shutil.ignore_patterns('media', 'addr.json'))
r = subprocess.run([P, 'fetch-media', '--dir', desk, '--from', f'{W}/home-lan.json', '--hash', hashes['B']], capture_output=True, text=True, env=lan_env, timeout=900)
ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
res['4_lan_only']['media'] = ev[-1] if ev else r.stderr[-300:]
print(json.dumps(res), flush=True)
