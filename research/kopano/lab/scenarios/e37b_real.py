"""E37b (real machines, roles reversed because the laptop helper only accepts single file names):
L) the desktop serves LAN-only (no relay, no lookup), the home laptop fetches original A over Wi-Fi;
X) Soshanguve fetches original B from the home laptop (laptop to laptop, different networks);
S) the desktop (fresh folder) fetches B from Soshanguve alone, then swarms B from both laptops.
Usage: python e37b_real.py <bin dir> <alice dir> <work dir>  (after e37_real.py made A and B)"""
import json, os, shutil, subprocess, sys, time

BIN, ALICE, W = sys.argv[1:4]
P, AG = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'agent.exe')
ID = {'home': '52f7a285291c0bf09d80cfa3243724379318466ab342bff422170710e05029d8',
      'sosh': 'f2189abd27ffe41cd186316176d58634e0ecaca5efd56e74ef2677b90862651e'}
H = {n: json.loads(subprocess.run([P, 'add-media', '--dir', ALICE, '--file', f'{W}/orig{n}.bin'], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash'] for n in 'AB'}

def call(who, req, timeout=1800):
    last = ''
    for _ in range(3):
        r = subprocess.run([AG, 'call', '--dir', ALICE, '--to', ID[who], '--json', json.dumps(req)], capture_output=True, text=True, timeout=timeout + 120)
        lines = [l for l in r.stdout.splitlines() if l.startswith('{')]
        if lines:
            return json.loads(lines[-1])
        last = r.stderr[-300:]
        time.sleep(5)
    return {'error': last}

def run(who, args, files=None, timeout=1800):
    out = call(who, {'op': 'run', 'args': args, 'files': files or {}, 'timeout_s': timeout}, timeout)
    res = out.get('reply', {}).get('result', {})
    ev = [json.loads(l) for l in res.get('stdout', '').splitlines() if l.startswith('{')]
    return ev[-1] if ev else {'err': res.get('stderr', '')[-300:] or out.get('reply', {}).get('error') or out.get('error')}

def media_row(ev):
    if 'err' in ev:
        return ev
    return {'path': ev.get('path'), 's': round(ev.get('ms', 0) / 1000, 1), 'mbit_s': ev.get('mbit_s'), 'mb': round(ev.get('bytes_received', 0) / 1e6, 1), 'complete': ev.get('complete') or ev.get('already_complete')}

res = {}
# L) desktop LAN-only server; home fetches A
addr = os.path.join(ALICE, 'addr.json')
if os.path.exists(addr):
    os.remove(addr)
srv = subprocess.Popen([P, 'serve', '--dir', ALICE], env=dict(os.environ, KOPANO_LAN_ONLY='1'), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(addr):
    time.sleep(0.3)
lan = open(addr).read()
res['L_desktop_lan_addr_has_relay'] = 'Relay' in lan
res['L_home_gets_A_over_wifi'] = media_row(run('home', ['fetch-media', '--from', 'd.json', '--hash', H['A']], {'d.json': lan}, 900))
srv.kill()
print(json.dumps(res), flush=True)
# X) laptop to laptop
home_addr = call('home', {'op': 'serve-addr'}).get('reply', {}).get('result', {}).get('addr', '')
res['X_sosh_gets_B_from_home'] = media_row(run('sosh', ['fetch-media', '--from', 'h.json', '--hash', H['B']], {'h.json': home_addr}))
print(json.dumps(res), flush=True)
# S) desktop as the receiver: Soshanguve alone, then both laptops at once
time.sleep(20)  # let the helpers' servers restart
home_addr = call('home', {'op': 'serve-addr'}).get('reply', {}).get('result', {}).get('addr', '')
sosh_addr = call('sosh', {'op': 'serve-addr'}).get('reply', {}).get('result', {}).get('addr', '')
open(f'{W}/home.json', 'w').write(home_addr)
open(f'{W}/sosh.json', 'w').write(sosh_addr)
for name, src in (('S1_desktop_from_sosh_only', [f'{W}/sosh.json']), ('S2_desktop_swarm_home_and_sosh', [f'{W}/home.json', f'{W}/sosh.json'])):
    d = f'{W}/desk-{name[:2]}'
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(ALICE, d, ignore=shutil.ignore_patterns('media', 'addr.json'))
    r = subprocess.run([P, 'fetch-swarm', '--dir', d, '--from', ','.join(src), '--hash', H['B']], capture_output=True, text=True, timeout=1800)
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    res[name] = ev[-1] if ev else {'err': r.stderr[-300:]}
    print(json.dumps(res), flush=True)
