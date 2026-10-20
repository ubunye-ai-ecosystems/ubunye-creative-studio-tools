"""S8 clash pick: deciding a clash with the panel's own buttons, two real Light windows, each
with its own kopanod (09 section 9, case 5). Buttons are pressed with real egui input
(ui.clickWidget), not through the service API.

A. A library shared live ("Work together live"): Alice and Member1 change the same setting of the
   same photo at the same moment. Both panels list the clash with one button per value.
   1. Alice presses the button with her value (keeps hers): both windows end on it, no clash left
   2. again on another setting; Member1 presses the button with Alice's value (takes hers)
B. An "I approve changes" library: Member1 and Alice change the same setting at once. Alice's
   "Changes to review" shows both values with "Keep yours" / "Take theirs".
   3. Keep yours, Approve: main keeps her value, his other change comes in
   4. again; Take theirs, Approve: main takes his value
Usage: python s8_clash_pick.py <lightcraft.exe> <kopanod.exe> <work dir> <alice id> <m1 id> --photos DIR
       [--shots DIR] [--light-base 18560] [--port-base 47560]"""
import sys, threading, time
from s8_common import Run, Kopanod, Light, arg, cert, fresh_homes, key, shots, wait_for

LIGHT, K, W, ALICE, M1 = sys.argv[1:6]
PHOTOS = arg('--photos')
LB, KB = int(arg('--light-base', 18560)), int(arg('--port-base', 47560))
run = Run('s8_clash_pick', W)
m1, adev = cert(M1), cert(ALICE)['device']


def at_once(*fns):
    th = [threading.Thread(target=f) for f in fns]
    [t.start() for t in th]; [t.join() for t in th]


def click(l, wid, tries=30):
    """Press a panel button with real input; scroll the panel to it when it is not on screen."""
    for i in range(tries):
        ws = l.rpc('ui.widgets', {'filter': wid.rstrip('*')})
        ws = ws if isinstance(ws, list) else ws.get('widgets', [])
        hit = [w['id'] for w in ws if w.get('id') == 'button:' + wid or (wid.endswith('-*') and w.get('id', '').startswith('button:' + wid[:-1]))]
        if hit:
            l.rpc('ui.clickWidget', {'id': hit[0]})
            return True
        time.sleep(0.5)
    seen = l.rpc('ui.widgets', {'filter': 'kopano-'})
    seen = seen if isinstance(seen, list) else seen.get('widgets', [])
    run.res.setdefault('missing_widgets', []).append({'window': l.who, 'wanted': wid, 'seen': [w.get('id') for w in seen][:60]})
    return False


def setup(tag, mode):
    fresh_homes(f'{run.W}/{tag}', {'a': ALICE, 'b': M1})
    sub = Run(run.name, f'{run.W}/{tag}')
    sub.procs = run.procs
    base = 0 if mode == 'live' else 10
    ka, kb = Kopanod(sub, 'a', K, KB + base + 1).start(), Kopanod(sub, 'b', K, KB + base + 2).start()
    la = Light(sub, 'a', LIGHT, LB + base + 1, PHOTOS, ka).start()
    lb = Light(sub, 'b', LIGHT, LB + base + 2, PHOTOS, kb).start()
    la.run = lb.run = run  # screenshots and results go to the scenario
    time.sleep(10)
    made = ka.call(op='create-project', name=f'Owner shoot ({mode})', mode=mode, app={'kind': 'light-control', 'port': la.port},
                   members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    kb.call(op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': lb.port})
    run.check(f'{mode}: joined', wait_for(lambda: kb.call(op='status', project=pid)['head'] is not None and len(lb.photos()) == len(la.photos()), 60) is not None)
    time.sleep(3)
    for l in (la, lb):
        l.cmd('kopano.panel', open=True)
        l.rpc('ui.set', {'view': 'detail'})
    return ka, kb, la, lb, pid


try:
    # ---- A. live ----
    ka, kb, la, lb, pid = setup('live', 'live')
    st = lambda k: k.call(op='status', project=pid)
    ph = sorted(la.photos(), key=lambda p: str(p.get('fileName')))
    X = key(ph[0])
    ia, ib = la.id_of(X), lb.id_of(X)
    la.cmd('library.select', ids=[ia], active=ia)
    lb.cmd('library.select', ids=[ib], active=ib)
    tag = pid[:8]
    for n, (ctl, va, vb, presser, want_who) in enumerate((('light.exposure', 0.6, -0.6, 'a', 'a'), ('light.contrast', 18, -18, 'b', 'a'))):
        clash = None
        for attempt in range(6):
            at_once(lambda: la.cmd('develop.set', control=ctl, value=va + attempt * 0.01 * (1 if va > 0 else -1), ids=[ia]),
                    lambda: lb.cmd('develop.set', control=ctl, value=vb - attempt * 0.01 * (1 if vb < 0 else -1), ids=[ib]))
            wait_for(lambda: st(ka)['clashes'] and st(kb)['clashes'] and st(ka)['head'] == st(kb)['head'], 15)
            cs = [c for c in st(ka)['clashes'] if any(x['setting'].endswith(ctl.split('.')[1]) for x in c['settings'])]
            if cs:
                clash = cs[0]
                break
            time.sleep(2)
        run.res.setdefault('clash_attempts', []).append(attempt + 1)
        run.check(f'A{n + 1} a clash came from editing at once', clash is not None, f'{attempt + 1} attempt(s)')
        if clash is None:
            continue
        run.check(f'A{n + 1} both panels list it', bool(st(kb)['clashes']))
        s0 = next(s for s in clash['settings'] if s['setting'].endswith(ctl.split('.')[1]))
        a_value = va + attempt * 0.01 * (1 if va > 0 else -1)
        pick = 'A' if abs(s0['a'] - a_value) < 1e-6 else 'B'
        time.sleep(1.5)
        shots(run, (la, lb), f'A{n + 1}a-live-clash', f'Both changed {ctl.split(".")[1]} at once: both panels list the clash with one button per value')
        l = la if presser == 'a' else lb
        pressed = click(l, f'kopano-pick{pick}-{tag}-0')
        run.check(f'A{n + 1} {"Alice" if presser == "a" else "Member1"} pressed the button with Alice\'s value', pressed, f'kopano-pick{pick}-{tag}-0')
        ok = wait_for(lambda: abs(la.dev(ia, ctl) - a_value) < 1e-6 and abs(lb.dev(ib, ctl) - a_value) < 1e-6 and not st(ka)['clashes'] and not st(kb)['clashes'], 30)
        run.check(f'A{n + 1} both windows on the picked value, no clash left', ok is not None,
                  {'a': la.dev(ia, ctl), 'b': lb.dev(ib, ctl), 'want': a_value, 'clashes': [len(st(ka)['clashes']), len(st(kb)['clashes'])], 'ms': ok})
        run.check(f'A{n + 1} same head', wait_for(lambda: st(ka)['head'] == st(kb)['head'], 20) is not None)
        time.sleep(1.5)
        shots(run, (la, lb), f'A{n + 1}b-live-picked', 'Picked: both windows show Alice\'s value, the clash is gone')
    for p in list(run.procs):
        p.terminate()
    for p in list(run.procs):
        p.wait()
    run.procs.clear()

    # ---- B. approve ----
    ka, kb, la, lb, pid = setup('approve', 'approve')
    st = lambda k: k.call(op='status', project=pid)
    rv = lambda k: st(k)['review']
    ph = sorted(la.photos(), key=lambda p: str(p.get('fileName')))
    X, Y = key(ph[1]), key(ph[2])
    ia, ib, ia2, ib2 = la.id_of(X), lb.id_of(X), la.id_of(Y), lb.id_of(Y)
    la.cmd('library.select', ids=[ia], active=ia)
    lb.cmd('library.select', ids=[ib], active=ib)
    tag = pid[:8]
    photo_tag = X[:8]
    for n, (ctl, va, vb, button, want) in enumerate((('light.exposure', 0.5, -0.5, 'keepMain', 'main'), ('light.contrast', 15, -15, 'takeMember', 'member'))):
        clash = None
        for attempt in range(6):
            d = attempt * 0.01
            a_value, b_value = va + d, vb - d
            # Member1 also changes another photo, which must come in either way
            lb.cmd('develop.set', control='light.highlights' if n == 0 else 'light.shadows', value=10 + attempt, ids=[ib2])
            at_once(lambda: la.cmd('develop.set', control=ctl, value=a_value, ids=[ia]), lambda: lb.cmd('develop.set', control=ctl, value=b_value, ids=[ib]))
            got = wait_for(lambda: any(p['clashes'] for p in rv(ka)['pending']), 15)
            mine = [p for p in rv(ka)['pending'] if p['clashes']]
            if mine:
                clash = mine[0]
                break
            run.res.setdefault('approve_no_clash', []).append({'attempt': attempt, 'pending': rv(ka)['pending'], 'member_value': lb.dev(ib, ctl), 'owner_value': la.dev(ia, ctl)})
            # no clash shown: settle this round before the next try
            if rv(ka)['pending']:
                ka.call(op='decline', project=pid, member=m1['account'])
            time.sleep(2)
        run.res.setdefault('approve_clash_attempts', []).append(attempt + 1)
        run.check(f'B{n + 1} the clash shows in her review', clash is not None, f'{attempt + 1} attempt(s)')
        if clash is None:
            continue
        cs = clash['clashes'][0]
        sv = next((s for s in cs['settings'] if s['setting'].endswith(ctl.split('.')[1])), None)
        run.check(f'B{n + 1} both values listed', sv is not None and abs(sv['main'] - a_value) < 1e-6 and abs(sv['member'] - b_value) < 1e-6, sv)
        time.sleep(1.5)
        shots(run, (la, lb), f'B{n + 1}a-review-clash', f'Changes to review: both values with Keep yours / Take theirs')
        wid = f'kopano-{button}-{tag}-*'  # one clash: the photo part is the service's short photo id
        run.check(f'B{n + 1} pressed {"Keep yours" if want == "main" else "Take theirs"}', click(la, wid), wid)
        time.sleep(0.5)
        time.sleep(1)
        run.res.setdefault('approve_before', []).append({'pending': rv(ka)['pending'], 'member_value': lb.dev(ib, ctl)})
        run.check(f'B{n + 1} pressed Approve', click(la, f'kopano-approve-{tag}-{m1["account"]}'))
        want_v = a_value if want == 'main' else b_value
        other = 'light.highlights' if n == 0 else 'light.shadows'
        ok = wait_for(lambda: abs(la.dev(ia, ctl) - want_v) < 1e-6 and la.dev(ia2, other) == 10 + attempt, 30)
        run.check(f'B{n + 1} main has the chosen value and his other change', ok is not None, {'value': la.dev(ia, ctl), 'want': want_v, other: la.dev(ia2, other)})
        ok2 = wait_for(lambda: abs(lb.dev(ib, ctl) - want_v) < 1e-6 and rv(kb)['mine']['state'] == 'approved', 30)
        run.check(f'B{n + 1} his window is main again', ok2 is not None, {'value': lb.dev(ib, ctl), 'state': rv(kb)['mine']['state']})
        time.sleep(1.5)
        shots(run, (la, lb), f'B{n + 1}b-approved', f'Approved with {"Keep yours" if want == "main" else "Take theirs"}')
    run.res['status'] = {'a': st(ka), 'b': st(kb)}
finally:
    code = run.finish()
sys.exit(code)
