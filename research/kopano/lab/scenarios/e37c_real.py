"""E37c: the three real machines with test helper v8 (this desktop = Alice; home laptop on the
same Wi-Fi; Soshanguve laptop on another network, about 45 km away).

v8 changes the way steps run: `start` returns at once, the step keeps running on the laptop
whatever happens to the control link, and this driver polls `job` until it is done. Every reply,
including failures, goes to the log (E37's driver dropped failed calls, so step 2's cause was lost).

Steps (50 MB originals A, B, C on the desktop, BBRv3 everywhere):
  0 preflight: helper version, network type and firewall rules on each laptop
  1 home <- B from the desktop
  2 sosh <- A from the desktop
  3 sosh <- B from home only (laptop to laptop, different networks)
  4 home <- C from the desktop, then sosh swarms C from desktop + home (several address files)
  5 LAN-only: (a) home serves with no relay and no lookup, the desktop fetches B from it;
             (b) the desktop serves LAN-only, home fetches A from it
  6 live edits: desktop edits, both laptops pull; sosh edits, home pulls from sosh, the desktop
    pulls from home; all three heads must match
Usage: python e37c_real.py <bin dir> <alice dir> <work dir> [steps, e.g. 0,1,2] [--only home]"""
import json, os, shutil, subprocess, sys, time

BIN, ALICE, W = sys.argv[1:4]
STEPS = set((sys.argv[4] if len(sys.argv) > 4 and not sys.argv[4].startswith('--') else '0,1,2,3,4,5,6').split(','))
ONLY = sys.argv[sys.argv.index('--only') + 1] if '--only' in sys.argv else None
MB = 50
P, AG = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'agent.exe')
ID = {'home': '52f7a285291c0bf09d80cfa3243724379318466ab342bff422170710e05029d8',
      'sosh': 'f2189abd27ffe41cd186316176d58634e0ecaca5efd56e74ef2677b90862651e'}
WHO = [ONLY] if ONLY else ['home', 'sosh']
CC = {'KOPANO_CC': 'bbr'}
os.makedirs(W, exist_ok=True)
log = open(f'{W}/e37c.log', 'a')
res = {}


def note(**kw):
    log.write(json.dumps({'t': time.strftime('%H:%M:%S'), **kw}) + '\n')
    log.flush()


def show():
    print(json.dumps(res), flush=True)
    open(f'{W}/e37c.json', 'w').write(json.dumps(res, indent=1))


def call(who, req, tries=3):
    """One control request; returns the helper's reply ({'ok':..}) or {'ok': False, 'error': ..}."""
    last = ''
    for _ in range(tries):
        try:
            r = subprocess.run([AG, 'call', '--dir', ALICE, '--to', ID[who], '--json', json.dumps(req)], capture_output=True, text=True, timeout=180)
            lines = [l for l in r.stdout.splitlines() if l.startswith('{')]
            if lines:
                out = json.loads(lines[-1])
                note(who=who, op=req.get('op'), job=req.get('job'), path=out.get('control_path'), ms=out.get('total_ms'),
                     ok=out['reply'].get('ok'), error=out['reply'].get('error'))
                return out['reply']
            last = (r.stdout + r.stderr)[-400:]
        except subprocess.TimeoutExpired:
            last = 'control call timed out'
        note(who=who, op=req.get('op'), job=req.get('job'), failed=last)
        time.sleep(5)
    return {'ok': False, 'error': last}


def events(text):
    return [json.loads(l) for l in (text or '').splitlines() if l.startswith('{')]


def step(who, args, files=None, env=None, timeout=1800):
    """Start a step on a laptop and poll until it is done. Poll failures are tolerated for up to
    10 minutes in a row: the step itself keeps running on the laptop."""
    r = call(who, {'op': 'start', 'args': args, 'files': files or {}, 'env': {**CC, **(env or {})}, 'timeout_s': timeout})
    if not r.get('ok'):
        return {'err': r.get('error')}
    job, t, lost_since = r['result']['job'], time.time(), None
    while True:
        time.sleep(3)
        r = call(who, {'op': 'job', 'job': job, 'tail': 200000}, tries=1)
        if not r.get('ok'):
            lost_since = lost_since or time.time()
            if time.time() - lost_since > 600:
                return {'err': f'lost the laptop for 10 min during step {job}: {r.get("error")}'}
            continue
        lost_since = None
        v = r['result']
        if v['state'] == 'done':
            ev = events(v.get('stdout'))
            out = ev[-1] if ev else {}
            if v.get('code') != 0:
                out = {**out, 'err': (v.get('stderr') or '')[-400:], 'code': v.get('code')}
            elif not ev:
                out = {'first_line': (v.get('stdout') or '').splitlines()[:1]}
            out['wall_s'] = round(time.time() - t, 1)
            return out
        if time.time() - t > timeout + 600:
            return {'err': f'step {job} still {v["state"]} after {timeout + 600} s'}


def media_row(ev):
    if 'err' in ev and not ev.get('complete'):
        return ev
    return {'path': ev.get('path'), 's': round(ev.get('ms', 0) / 1000, 1), 'mbit_s': ev.get('mbit_s'),
            'mb': round(ev.get('bytes_received', 0) / 1e6, 1), 'complete': ev.get('complete') or ev.get('already_complete')}


def addr_of(who):
    for _ in range(20):  # the helper restarts its server after each step
        r = call(who, {'op': 'serve-addr'})
        a = r.get('result', {}).get('addr', '') if r.get('ok') else ''
        if a:
            return a
        time.sleep(3)
    return ''


def local(args, env=None, timeout=1800):
    r = subprocess.run([P, *args], capture_output=True, text=True, env=dict(os.environ, **CC, **(env or {})), timeout=timeout)
    ev = events(r.stdout)
    if r.returncode != 0:
        return {'err': r.stderr[-400:], 'code': r.returncode}
    return ev[-1] if ev else {'first_line': r.stdout.splitlines()[:1]}


def serve_local(d, env=None):
    a = os.path.join(d, 'addr.json')
    if os.path.exists(a):
        os.remove(a)
    p = subprocess.Popen([P, 'serve', '--dir', d], env=dict(os.environ, **CC, **(env or {})), stdout=subprocess.DEVNULL, stderr=open(f'{W}/serve-{len(os.listdir(W))}.log', 'w'))
    while not os.path.exists(a):
        if p.poll() is not None:
            raise SystemExit('local serve stopped')
        time.sleep(0.3)
    time.sleep(0.5)
    return p, open(a).read()


def fresh_copy(name):
    d = f'{W}/{name}'
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(ALICE, d, ignore=shutil.ignore_patterns('media', 'addr.json'))
    return d


# originals on the desktop
H = {}
for name in 'ABC':
    f = f'{W}/orig{name}.bin'
    if not os.path.exists(f):
        with open(f, 'wb') as o:
            for _ in range(MB):
                o.write(os.urandom(1 << 20))
    H[name] = local(['add-media', '--dir', ALICE, '--file', f])['hash']

if '0' in STEPS:
    for who in WHO:
        info = call(who, {'op': 'info'})
        net = call(who, {'op': 'net'})
        res[f'0_{who}'] = {'helper': info.get('result', {}).get('helper'), 'os': info.get('result', {}).get('os'), 'net': net.get('result') or net.get('error')}
        # every laptop serves with the same settings as this desktop
        call(who, {'op': 'serve', 'relay_only': False, 'env': CC})
    show()

# the desktop serves on a different relay than the control tool (one identity, two programs)
alice, alice_addr = serve_local(ALICE, {'KOPANO_RELAY_URL': 'https://use1-1.relay.n0.iroh.link./'})
try:
    if '1' in STEPS and 'home' in WHO:
        res['1_home_gets_B_from_desktop'] = media_row(step('home', ['fetch-media', '--from', 'a.json', '--hash', H['B']], {'a.json': alice_addr}))
        show()
    if '2' in STEPS and 'sosh' in WHO:
        res['2_sosh_gets_A_from_desktop'] = media_row(step('sosh', ['fetch-media', '--from', 'a.json', '--hash', H['A']], {'a.json': alice_addr}))
        show()
    if '3' in STEPS and {'home', 'sosh'} <= set(WHO):
        res['3_sosh_gets_B_from_home'] = media_row(step('sosh', ['fetch-media', '--from', 'h.json', '--hash', H['B']], {'h.json': addr_of('home')}))
        show()
    if '4' in STEPS and 'home' in WHO:
        res['4a_home_gets_C_from_desktop'] = media_row(step('home', ['fetch-media', '--from', 'a.json', '--hash', H['C']], {'a.json': alice_addr}))
        if 'sosh' in WHO:
            res['4b_sosh_swarms_C_from_desktop_and_home'] = step('sosh', ['fetch-swarm', '--from', 'a.json,h.json', '--hash', H['C']], {'a.json': alice_addr, 'h.json': addr_of('home')})
        show()
finally:
    alice.kill()

if '5' in STEPS and 'home' in WHO:
    lan = {'KOPANO_LAN_ONLY': '1'}
    # a) home serves LAN-only; a fresh desktop copy fetches B from it over the Wi-Fi
    call('home', {'op': 'serve', 'relay_only': False, 'env': {**CC, **lan}})
    home_lan = addr_of('home')
    open(f'{W}/home-lan.json', 'w').write(home_lan)
    d = fresh_copy('desk-lan')
    res['5a_desktop_gets_B_from_home_lan_only'] = {'home_addr_has_relay': 'Relay' in home_lan, **media_row(local(['fetch-media', '--dir', d, '--from', f'{W}/home-lan.json', '--hash', H['B']], lan, 900))}
    call('home', {'op': 'serve', 'relay_only': False, 'env': CC})
    show()
    # b) the desktop serves LAN-only; home fetches A from it (home already has A? then it says so)
    srv, desk_lan = serve_local(ALICE, lan)
    try:
        res['5b_home_gets_A_from_desktop_lan_only'] = {'desk_addr_has_relay': 'Relay' in desk_lan, **media_row(step('home', ['fetch-media', '--from', 'd.json', '--hash', H['A']], {'d.json': desk_lan}, lan, 900))}
    finally:
        srv.kill()
    show()

if '6' in STEPS:
    key = 'canon-5d4-raw.CR2'
    v = str(round(time.time() % 1000 / 1000 - 0.5, 3))
    local(['edit', '--dir', ALICE, '--key', key, '--field', 'develop.exposure', '--value', v])
    alice, alice_addr = serve_local(ALICE, {'KOPANO_RELAY_URL': 'https://use1-1.relay.n0.iroh.link./'})
    try:
        for who in WHO:
            res[f'6a_{who}_pulls_desktop_edit'] = step(who, ['pull', '--from', 'a.json', '--compact'], {'a.json': alice_addr})
        show()
    finally:
        alice.kill()
    last = WHO[-1]
    res[f'6b_{last}_edits'] = step(last, ['edit', '--key', key, '--field', 'develop.contrast', '--value', str(int(time.time()) % 50)])
    src = last
    if len(WHO) > 1:
        res['6c_home_pulls_from_sosh'] = step('home', ['pull', '--from', 's.json', '--compact'], {'s.json': addr_of('sosh')})
        src = 'home'
    open(f'{W}/src.json', 'w').write(addr_of(src))
    res[f'6d_desktop_pulls_from_{src}'] = local(['pull', '--dir', ALICE, '--from', f'{W}/src.json', '--compact'])
    heads = {'desktop': local(['status', '--dir', ALICE])}
    for who in WHO:
        heads[who] = step(who, ['status'])
    res['6e_heads'] = {k: ''.join(h.get('first_line', [])).rsplit('head ', 1)[-1] or h for k, h in heads.items()}
    res['6e_all_equal'] = len({json.dumps(h) for h in res['6e_heads'].values()}) == 1
    show()
