"""E39: why big files crawl to the Soshanguve laptop (E37c step 2: 50 MB not done in 30 min while a
977-byte edit took 0.8 s). Short 5 MB fetches, one setting changed at a time, with test helper v8
as it is on the laptop (nothing new to install there):
  a  desktop serves normally (relay + direct attempts), as in E37c
  b  desktop serves relay-only: every byte through the relay, no direct attempts at all
  c  the laptop fetches with --relay-only as well
Usage: python e39_direct.py <bin dir> <alice dir> <work dir> <who> <laptop id> [cases, e.g. a,b,c] [--mb N] [--relay URL]"""
import json, os, subprocess, sys, time

BIN, ALICE, W, WHO, LID = sys.argv[1:6]
CASES = (sys.argv[6] if len(sys.argv) > 6 and not sys.argv[6].startswith('--') else 'a,b,c').split(',')
MB = int(sys.argv[sys.argv.index('--mb') + 1]) if '--mb' in sys.argv else 5
RELAY = sys.argv[sys.argv.index('--relay') + 1] if '--relay' in sys.argv else 'https://use1-1.relay.n0.iroh.link./'
P, AG = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'agent.exe')
CC = {'KOPANO_CC': 'bbr'}
os.makedirs(W, exist_ok=True)
log = open(f'{W}/e39.log', 'a')


def note(**kw):
    log.write(json.dumps({'t': time.strftime('%H:%M:%S'), **kw}) + '\n')
    log.flush()


def call(req):
    r = subprocess.run([AG, 'call', '--dir', ALICE, '--to', LID, '--json', json.dumps(req)], capture_output=True, text=True, timeout=180)
    lines = [l for l in r.stdout.splitlines() if l.startswith('{')]
    return json.loads(lines[-1])['reply'] if lines else {'ok': False, 'error': (r.stdout + r.stderr)[-300:]}


def step(args, files, timeout):
    r = call({'op': 'start', 'args': args, 'files': files, 'env': CC, 'timeout_s': timeout})
    if not r.get('ok'):
        return {'err': r.get('error')}
    job, t = r['result']['job'], time.time()
    while True:
        time.sleep(3)
        try:
            v = call({'op': 'job', 'job': job, 'tail': 20000})
        except subprocess.TimeoutExpired:
            continue
        if v.get('ok') and v['result']['state'] == 'done':
            v = v['result']
            ev = [json.loads(l) for l in (v.get('stdout') or '').splitlines() if l.startswith('{')]
            out = ev[-1] if ev else {}
            if v.get('code') != 0:
                out = {**out, 'err': (v.get('stderr') or '')[-300:]}
            out['wall_s'] = round(time.time() - t, 1)
            return out


def sent_bytes():
    o = subprocess.run(['powershell', '-NoProfile', '-Command', '(Get-NetAdapterStatistics | Measure-Object SentBytes -Sum).Sum'], capture_output=True, text=True).stdout
    return int(o.strip() or 0)


def serve(env, relay_only):
    a = os.path.join(ALICE, 'addr.json')
    if os.path.exists(a):
        os.remove(a)
    cmd = [P, 'serve', '--dir', ALICE] + (['--relay-only'] if relay_only else [])
    p = subprocess.Popen(cmd, env=dict(os.environ, **CC, **env), stdout=subprocess.DEVNULL, stderr=open(f'{W}/serve-{time.time():.0f}.log', 'w'))
    while not os.path.exists(a):
        if p.poll() is not None:
            raise SystemExit('serve stopped')
        time.sleep(0.3)
    time.sleep(0.5)
    return p, open(a).read()


res = {}
for case in CASES:
    f = f'{W}/d{case}.bin'  # a fresh original per case, so nothing is already on the laptop
    with open(f, 'wb') as o:
        o.write(os.urandom(MB << 20))
    h = json.loads([l for l in subprocess.run([P, 'add-media', '--dir', ALICE, '--file', f], capture_output=True, text=True).stdout.splitlines() if l.startswith('{')][-1])['hash']
    p, addr = serve({'KOPANO_RELAY_URL': RELAY}, relay_only=(case in 'bc'))
    args = ['fetch-media', '--from', 'a.json', '--hash', h] + (['--relay-only'] if case == 'c' else [])
    s0, t0 = sent_bytes(), time.time()
    out = step(args, {'a.json': addr}, timeout=300)
    up = (sent_bytes() - s0) * 8 / max(time.time() - t0, 1) / 1e6
    p.kill()
    res[case] = {'path': out.get('path'), 'complete': out.get('complete'), 's': round(out.get('ms', 0) / 1000, 1), 'mbit_s': out.get('mbit_s'),
                 'desktop_upload_mbit_s': round(up, 2), 'wall_s': out.get('wall_s'), 'err': out.get('err')}
    note(case=case, mb=MB, relay=RELAY, **res[case])
    print(case, json.dumps(res[case]), flush=True)
open(f'{W}/e39.json', 'w').write(json.dumps(res, indent=1))
