"""S3a: Kopano inside the real Light app. Two Light windows on this PC (throwaway demo libraries,
`--memory`), each with its own kopanod and device identity. Alice starts a project from her Light;
Member1 joins it into his Light; edits made IN Light travel to the other Light with no command.

Edits are made through Light's control channel the way its own UI makes them (develop.set,
photo.rate on the selected photo), and once with a real slider drag (ui.dragWidget).
Usage: python s3_light.py <lightcraft.exe> <kopanod.exe> <work dir> <alice lab dir> <m1 lab dir>"""
import json, os, shutil, socket, subprocess, sys, time

LIGHT, K, W, ALICE, M1 = sys.argv[1:6]
LP, KP = {'a': 18001, 'b': 18002}, {'a': 47111, 'b': 47112}
res = {}


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
    s = socket.create_connection(('127.0.0.1', KP[who]), timeout=120)
    s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
    r = json.loads(s.makefile().readline())
    s.close()
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


def key(p):
    return p.get('content_hash') or f"{p.get('file_name')}|{p.get('captured')}"


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


shutil.rmtree(W, ignore_errors=True)
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
procs = []
try:
    for who in 'ab':
        procs.append(subprocess.Popen([LIGHT, '--memory', '--control', str(LP[who])], stdout=subprocess.DEVNULL, stderr=open(f'{W}/light-{who}.log', 'w')))
        log = open(f'{W}/{who}/kopanod.log', 'w')
        procs.append(subprocess.Popen([K, 'run', '--home', f'{W}/{who}', '--api-port', str(KP[who])], stdout=log, stderr=log))
    la, lb = Light(LP['a']), Light(LP['b'])
    pa, pb = la.photos(), lb.photos()
    res['photos'] = {'a': len(pa), 'b': len(pb), 'same_keys': sorted(map(key, pa)) == sorted(map(key, pb))}
    while not os.path.exists(f'{W}/b/api.token'):
        time.sleep(0.2)
    time.sleep(12)  # both services online
    m1 = json.load(open(f'{W}/b/cert.json'))['body']
    bdev = m1['device']
    adev = json.load(open(f'{W}/a/cert.json'))['body']['device']

    t0 = time.time()
    made = kcall('a', op='create-project', name='Light shoot (S3)', app={'kind': 'light-control', 'port': LP['a']},
                 members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    res['create'] = {'items': made['items'], 'ms': round((time.time() - t0) * 1000)}
    t0 = time.time()
    kcall('b', op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': LP['b']})
    res['join_ms'] = round((time.time() - t0) * 1000)

    target = pa[0]
    ida = target['id']
    idb = next(p['id'] for p in lb.photos() if key(p) == key(target))

    def exposure(l, i):
        return l.cmd('photo.inspect', id=i).get('develop', {}).get('light', {}).get('exposure')

    def shots(tag):
        for who, l in (('a', la), ('b', lb)):
            try:
                l.rpc('ui.screenshot', {'path': os.path.abspath(f'{W}/{tag}-{who}.png')})
            except Exception as e:
                res.setdefault('screenshot_errors', []).append(str(e))
    time.sleep(6)  # let the join's first sync land in Member1's Light
    # both windows show the same photo large, so the change is visible
    la.cmd('library.select', ids=[ida], active=ida)
    lb.cmd('library.select', ids=[idb], active=idb)
    for l in (la, lb):
        l.rpc('ui.set', {'view': 'detail'})
    time.sleep(2)
    shots('1-before')
    # 1. Alice edits exposure in her Light; it must show in Member1's Light
    t0 = time.time()
    la.cmd('develop.set', control='light.exposure', value=1.25, ids=[ida])
    res['a_to_b_exposure_ms'] = wait_for(lambda: abs((exposure(lb, idb) or 0) - 1.25) < 1e-6, 30)
    time.sleep(1.5)
    shots('2-after-alice-exposure')

    # 2. Member1 rates the photo in his Light; it must show in Alice's Light
    lb.cmd('library.select', ids=[idb], active=idb)
    lb.cmd('photo.rate', rating=5, ids=[idb])
    res['b_to_a_rating_ms'] = wait_for(lambda: la.cmd('photo.inspect', id=ida).get('rating') == 5, 30)
    time.sleep(1.5)
    shots('3-after-member1-rating')

    # 3. a real slider drag in Member1's window
    lb.rpc('ui.set', {'view': 'detail'})
    time.sleep(1)
    ws = lb.rpc('ui.widgets', {'filter': 'exposure'})
    ws = ws.get('widgets', []) if isinstance(ws, dict) else ws
    w = [x for x in ws if 'exposure' in x['id']]
    res['slider_widgets'] = [x['id'] for x in w][:5]
    if w:
        before = exposure(la, ida)
        lb.rpc('ui.dragWidget', {'id': w[0]['id'], 'dx': 60, 'steps': 6})
        after_b = exposure(lb, idb)
        res['slider'] = {'b_value': after_b, 'arrived_in_a_ms': wait_for(lambda: exposure(la, ida) == after_b and after_b != before, 30)}
    res['status_a'] = kcall('a', op='status', project=pid)
    res['status_b'] = kcall('b', op='status', project=pid)
    res['same_head'] = res['status_a']['head'] == res['status_b']['head']
finally:
    for p in procs:
        p.terminate()
    print(json.dumps(res, indent=1))
