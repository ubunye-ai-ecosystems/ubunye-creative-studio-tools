"""S7: "I approve changes" with two real Light windows on this PC, each with its own kopanod
(own KOPANO_HOME and KOPANO_API_PORT) and device identity, on the owner's own photos.

Alice shares her library in approve mode; Member1 joins. Checked, with screenshots of both windows
(Share & Collaborate panel open) at every step:
  1. Member1 edits exposure and stars: his window shows it at once, Alice's main and window do not
     change, Alice's panel lists "Member1 changed 1 photo", his says "Waiting for Alice's approval"
  2. Alice presses View: her window shows his version (After), then hers (Before), then Done
  3. Alice approves: her window follows (exposure and stars arrive), his says "Approved"
  4. Member1 edits contrast, Alice declines: her main and window stay, his window keeps it
  5. Alice edits main meanwhile: Member1's window picks it up and keeps his own work
  6. Both kopanod are stopped and started again: review state, heads and windows are kept, and
     the next edit still goes for review
  7. Member1 (not an approver) tries to approve his own work: refused, main unchanged
Edits are made through Light's control channel the way its own UI makes them; the panel's
buttons run the same commands (kopano.view / kopano.approve / kopano.decline).
Usage: python s7_approve.py <lightcraft.exe> <kopanod.exe> <work dir> <alice id dir> <m1 id dir>
       --photos <folder> [--shots <dir>] [--light-base N] [--port-base N]"""
import json, os, shutil, socket, subprocess, sys, time

LIGHT, K, W, ALICE, M1 = sys.argv[1:6]
arg = lambda n, d=None: sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d
PHOTOS = arg('--photos')
SHOTS = arg('--shots', os.path.join(W, 'shots'))
LB, KB = int(arg('--light-base', 18400)), int(arg('--port-base', 47400))
LP, KP = {'a': LB + 1, 'b': LB + 2}, {'a': KB + 1, 'b': KB + 2}
res = {'steps': [], 'checks': {}}


class Light:
    def __init__(self, port):
        for _ in range(300):
            try:
                self.s = socket.create_connection(('127.0.0.1', port), timeout=60)
                break
            except OSError:
                time.sleep(0.2)
        self.f, self.n = self.s.makefile('r'), 0

    def rpc(self, method, params=None):
        self.n += 1
        self.s.sendall((json.dumps({'id': self.n, 'method': method, 'params': params or {}}) + '\n').encode())
        r = json.loads(self.f.readline())
        if not r.get('ok'):
            raise RuntimeError(f'{method}: {r.get("error")}')
        return r.get('result')

    def cmd(self, command, **params):
        return self.rpc('engine.execute', {'command': command, 'params': params})

    def photos(self):
        out = []
        while True:
            page = self.cmd('catalog.query', filter={}, offset=len(out), limit=500)
            items = page.get('photos', page) if isinstance(page, dict) else page
            out += items
            if len(items) < 500:
                return out


def kcall(who, **req):
    tok = open(f'{W}/{who}/api.token').read().strip()
    for _ in range(100):
        try:
            s = socket.create_connection(('127.0.0.1', KP[who]), timeout=120)
            break
        except OSError:
            time.sleep(0.2)
    s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
    r = json.loads(s.makefile().readline())
    s.close()
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


def key(p):
    return p.get('content_hash') or f"{p.get('fileName') or p.get('file_name')}|{p.get('captured')}"


def wait_for(fn, timeout):
    t0, end = time.time(), time.time() + timeout
    while time.time() < end:
        try:
            if fn():
                return round((time.time() - t0) * 1000)
        except Exception:
            pass
        time.sleep(0.1)
    return None


def check(name, ok, detail=None):
    res['checks'][name] = bool(ok) if detail is None else {'ok': bool(ok), 'detail': detail}
    print(('PASS ' if ok else 'FAIL ') + name + ('' if detail is None else f'  {detail}'), flush=True)


def start_kopanod(who):
    log = open(f'{W}/{who}/kopanod.log', 'a')
    return subprocess.Popen([K, 'run', '--home', f'{W}/{who}', '--api-port', str(KP[who])], stdout=log, stderr=log)


shutil.rmtree(W, ignore_errors=True)
os.makedirs(SHOTS, exist_ok=True)
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
lights, kopanods = {}, {}
try:
    for who in 'ab':
        env = {**os.environ, 'KOPANO_HOME': os.path.abspath(f'{W}/{who}'), 'KOPANO_API_PORT': str(KP[who])}
        lights[who] = subprocess.Popen([LIGHT, '--memory', '--control', str(LP[who]), '--no-demo', PHOTOS], env=env, stdout=subprocess.DEVNULL, stderr=open(f'{W}/light-{who}.log', 'w'))
        kopanods[who] = start_kopanod(who)
    la, lb = Light(LP['a']), Light(LP['b'])
    n = len(os.listdir(PHOTOS))
    for _ in range(120):  # imports run on a worker thread
        pa, pb = la.photos(), lb.photos()
        if len(pa) >= n and len(pb) == len(pa):
            break
        time.sleep(1)
    res['photos'] = {'a': len(pa), 'b': len(pb), 'same_keys': sorted(map(key, pa)) == sorted(map(key, pb))}
    while not os.path.exists(f'{W}/b/api.token'):
        time.sleep(0.2)
    time.sleep(12)  # both services online
    m1 = json.load(open(f'{W}/b/cert.json'))['body']
    adev = json.load(open(f'{W}/a/cert.json'))['body']['device']

    def shots(tag, caption):
        paths = {}
        for who, l in (('a', la), ('b', lb)):
            p = os.path.abspath(f'{SHOTS}/{tag}-{who}.png')
            try:
                l.rpc('ui.settle', {'timeoutMs': 4000})
            except Exception:
                pass
            try:
                l.rpc('ui.screenshot', {'path': p})
                paths[who] = p
            except Exception as e:
                res.setdefault('screenshot_errors', []).append(f'{tag}-{who}: {e}')
        res['steps'].append({'tag': tag, 'caption': caption, **paths})

    # 0. share in approve mode, join
    t0 = time.time()
    made = kcall('a', op='create-project', name='Owner shoot (approve)', mode='approve', app={'kind': 'light-control', 'port': LP['a']},
                 members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    res['create'] = {'items': made['items'], 'photos': made['photos'], 'mode': made.get('mode'), 'ms': round((time.time() - t0) * 1000)}
    check('created in approve mode', made.get('mode') == 'approve')
    t0 = time.time()
    kcall('b', op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': LP['b']})
    res['join_ms'] = round((time.time() - t0) * 1000)
    st = lambda who: kcall(who, op='status', project=pid)
    rv = lambda who: st(who)['review']
    joined = wait_for(lambda: st('b')['head'] == rv('a')['main'], 60)
    check('member window shows main after joining', joined is not None, f'{joined} ms')
    for l in (la, lb):
        l.cmd('kopano.panel', open=True)

    target = next((p for p in pa if str(p.get('fileName', '')).startswith('DSC08379')), pa[0])
    ida = target['id']
    idb = next(p['id'] for p in lb.photos() if key(p) == key(target))
    other = next(p for p in pa if p['id'] != ida)
    ida2 = other['id']
    idb2 = next(p['id'] for p in lb.photos() if key(p) == key(other))

    def dev(l, i, ctl):
        group, name = ctl.split('.')
        return l.cmd('photo.inspect', id=i).get('develop', {}).get(group, {}).get(name)

    def rating(l, i):
        return l.cmd('photo.inspect', id=i).get('rating')

    la.cmd('library.select', ids=[ida], active=ida)
    lb.cmd('library.select', ids=[idb], active=idb)
    for l in (la, lb):
        l.rpc('ui.set', {'view': 'detail'})
    time.sleep(2)
    shots('0-joined', 'Member1 joined Alice\'s "I approve changes" library: both windows show main')
    main0 = rv('a')['main']

    # 1. Member1 edits; Alice's main and window stay
    exp0, stars0 = dev(la, ida, 'light.exposure') or 0, rating(la, ida) or 0
    lb.cmd('develop.set', control='light.exposure', value=1.5, ids=[idb])
    lb.cmd('library.select', ids=[idb], active=idb)
    lb.cmd('photo.rate', rating=4, ids=[idb])
    check('member sees his own edit at once', dev(lb, idb, 'light.exposure') == 1.5 and rating(lb, idb) == 4)
    ms = wait_for(lambda: len(rv('a')['pending']) == 1 and rv('a')['pending'][0]['photos'] == 1, 30)
    res['pending_ms'] = ms
    p0 = rv('a')['pending'][0] if ms is not None else {}
    check('owner sees pending', ms is not None, f"{p0.get('name')} {p0.get('summary')} in {ms} ms")
    check('member says waiting', rv('b')['mine']['state'] == 'waiting', rv('b')['mine'])
    time.sleep(2)
    check('owner main unchanged', rv('a')['main'] == main0 and st('a')['head'] == main0)
    check('owner window unchanged', (dev(la, ida, 'light.exposure') or 0) == exp0 and (rating(la, ida) or 0) == stars0,
          {'exposure': dev(la, ida, 'light.exposure'), 'stars': rating(la, ida)})
    shots('1-member-edited', 'Member1 set exposure +1.5 and 4 stars: his window shows it, Alice\'s does not; her panel lists it for review')

    # 2. View: After, Before, Done
    la.cmd('kopano.view', project=pid, member=m1['account'], show='member')
    ms = wait_for(lambda: dev(la, ida, 'light.exposure') == 1.5, 20)
    check('view shows the member version in the owner window', ms is not None, f'{ms} ms')
    time.sleep(1.5)
    shots('2a-view-after', 'Alice pressed View: her window shows Member1\'s version (After)')
    la.cmd('kopano.view', project=pid, member=m1['account'], show='main')
    ms = wait_for(lambda: (dev(la, ida, 'light.exposure') or 0) == exp0, 20)
    check('before shows main again', ms is not None)
    time.sleep(1.5)
    shots('2b-view-before', 'Before: her own version')
    la.cmd('kopano.view', project=pid, show='off')
    time.sleep(2)
    check('looking saved nothing', rv('a')['main'] == main0 and rv('a')['viewing'] is None)

    # 3. approve
    t0 = time.time()
    la.cmd('kopano.approve', project=pid, member=m1['account'])
    ms = wait_for(lambda: dev(la, ida, 'light.exposure') == 1.5 and rating(la, ida) == 4, 30)
    res['approve_to_owner_window_ms'] = ms
    check('owner window follows the approval', ms is not None, f'{ms} ms')
    ms2 = wait_for(lambda: rv('b')['mine']['state'] == 'approved', 30)
    check('member says approved', ms2 is not None, f'{ms2} ms after')
    check('main moved', rv('a')['main'] != main0)
    ms3 = wait_for(lambda: st('b')['head'] == rv('a')['main'], 30)
    check('member window is main again', ms3 is not None)
    time.sleep(1.5)
    shots('3-approved', 'Alice approved: her window took exposure +1.5 and 4 stars; Member1\'s panel says Approved')
    main1 = rv('a')['main']

    # 4. decline
    lb.cmd('develop.set', control='light.contrast', value=30, ids=[idb])
    ms = wait_for(lambda: len(rv('a')['pending']) == 1, 30)
    check('second change pending', ms is not None)
    con0 = dev(la, ida, 'light.contrast') or 0
    la.cmd('kopano.decline', project=pid, member=m1['account'])
    ms = wait_for(lambda: rv('b')['mine']['state'] == 'declined', 30)
    check('member says declined', ms is not None, f'{ms} ms')
    time.sleep(2)
    check('decline: owner main unchanged', rv('a')['main'] == main1 and st('a')['head'] == main1)
    check('decline: owner window unchanged', (dev(la, ida, 'light.contrast') or 0) == con0, dev(la, ida, 'light.contrast'))
    check('decline: member keeps his work', dev(lb, idb, 'light.contrast') == 30)
    shots('4-declined', 'Alice declined Member1\'s contrast +30: her library stays, his window keeps it, his panel says Declined')

    # 5. owner edits main meanwhile; the member's view picks it up and keeps his own work
    la.cmd('develop.set', control='light.exposure', value=-0.7, ids=[ida2])
    ms = wait_for(lambda: dev(lb, idb2, 'light.exposure') == -0.7, 30)
    res['owner_edit_to_member_ms'] = ms
    check('owner edit reaches the member window', ms is not None, f'{ms} ms')
    check('member keeps his declined work', dev(lb, idb, 'light.contrast') == 30)
    check('owner main moved by her own edit', rv('a')['main'] != main1)
    time.sleep(2)
    check('declined work is not offered again when main moves', rv('b')['mine']['state'] == 'declined' and len(rv('a')['pending']) == 0,
          {'member': rv('b')['mine']['state'], 'pending': len(rv('a')['pending'])})
    lb.cmd('library.select', ids=[idb2], active=idb2)
    la.cmd('library.select', ids=[ida2], active=ida2)
    time.sleep(1.5)
    shots('5-owner-edit-reaches-member', 'Alice edited another photo (exposure -0.7) on main: it shows in Member1\'s window at once')
    lb.cmd('library.select', ids=[idb], active=idb)
    la.cmd('library.select', ids=[ida], active=ida)

    # 6. restart both kopanod
    before = {'ra': rv('a'), 'rb': rv('b'), 'ha': st('a')['head'], 'hb': st('b')['head']}
    for who in 'ab':
        kopanods[who].terminate()
        kopanods[who].wait()
    time.sleep(1)
    for who in 'ab':
        kopanods[who] = start_kopanod(who)
    ok = wait_for(lambda: st('a')['head'] == before['ha'] and st('b')['head'] == before['hb'], 60)
    after = {'ra': rv('a'), 'rb': rv('b')}
    check('restart: heads kept', ok is not None)
    check('restart: review kept', after['ra']['main'] == before['ra']['main'] and after['rb']['mine'] == before['rb']['mine'] and after['ra']['declined'] == before['ra']['declined']
          and after['rb']['mine']['state'] == 'declined', {'member': after['rb']['mine']['state']})
    time.sleep(8)  # online again
    lb.cmd('develop.set', control='light.contrast', value=35, ids=[idb])
    ms = wait_for(lambda: len(rv('a')['pending']) == 1 and rv('b')['mine']['state'] == 'waiting', 60)
    check('restart: the next edit goes for review', ms is not None, f'{ms} ms')
    check('restart: owner window unchanged', (dev(la, ida, 'light.contrast') or 0) == con0)
    time.sleep(1.5)
    shots('6-after-restart', 'Both services restarted: state kept; Member1\'s next edit (contrast 35) waits for review again')

    # 7. a member who is not an approver cannot move main
    m_before = rv('a')['main']
    try:
        kcall('b', op='approve', project=pid, member=m1['account'])
        refused = None
    except RuntimeError as e:
        refused = str(e)
    lb.cmd('kopano.approve', project=pid, member=m1['account'])  # through his panel too
    time.sleep(4)
    job = lb.cmd('kopano.status').get('job') or {}
    check('non-approver approve refused', refused is not None and 'only the owner' in refused.lower(), refused)
    check('panel shows the refusal', job.get('ok') is False, job.get('text'))
    check('main unchanged after the refused approve', rv('a')['main'] == m_before and rv('b')['main'] == m_before)
    shots('7-refused', 'Member1 pressed Approve on his own work: refused, Alice\'s main unchanged')

    res['status_a'] = st('a')
    res['status_b'] = st('b')
    res['panel_a'] = la.cmd('kopano.status')
    res['panel_b'] = lb.cmd('kopano.status')
finally:
    for p in list(kopanods.values()) + list(lights.values()):
        p.terminate()
    json.dump(res, open(os.path.join(W, 'result.json'), 'w'), indent=1)
    print(json.dumps({'checks': res['checks'], 'timings': {k: res.get(k) for k in ('join_ms', 'pending_ms', 'approve_to_owner_window_ms', 'owner_edit_to_member_ms')}}, indent=1))
    fails = [k for k, v in res['checks'].items() if not (v if isinstance(v, bool) else v['ok'])]
    print(f'{len(res["checks"]) - len(fails)}/{len(res["checks"])} checks passed' + (f'; failed: {fails}' if fails else ''))
