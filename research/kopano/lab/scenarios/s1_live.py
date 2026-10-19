"""S1: kopanod live sync on one PC, two services, two device identities (09-product-integration).

Done when: an edit on one side appears on the other within 2 s with no pull command; edits flow
both ways; concurrent edits (different items and the same setting) settle on one head on both
sides; a service that was stopped catches up when it comes back.
Usage: python s1_live.py <kopanod.exe> <home a> <home b> <b device id> [--edits N]"""
import json, os, socket, subprocess, sys, threading, time

K, A, B, BID = sys.argv[1:5]
N = int(sys.argv[sys.argv.index('--edits') + 1]) if '--edits' in sys.argv else 20
PORT = {A: 47101, B: 47102}
res = {}


def start(home):
    log = open(os.path.join(home, 'kopanod.log'), 'w')
    return subprocess.Popen([K, 'run', '--home', home, '--api-port', str(PORT[home])], stdout=log, stderr=log)


def token_of(home):
    f = os.path.join(home, 'api.token')
    for _ in range(300):
        if os.path.exists(f):
            break
        time.sleep(0.1)
    return open(f).read().strip()


def call(home, **req):
    token = token_of(home) or open(os.path.join(home, 'api.token')).read().strip()
    for _ in range(100):
        try:
            s = socket.create_connection(('127.0.0.1', PORT[home]), timeout=60)
            break
        except OSError:
            time.sleep(0.2)
    s.sendall((json.dumps({**req, 'token': token}) + '\n').encode())
    line = s.makefile().readline()
    s.close()
    r = json.loads(line)
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


class Events(threading.Thread):
    """An app panel: subscribes and keeps every event with the time it arrived."""

    def __init__(self, home):
        super().__init__(daemon=True)
        self.home, self.got, self.lock = home, [], threading.Lock()

    def run(self):
        token = token_of(self.home)
        while True:
            try:
                s = socket.create_connection(('127.0.0.1', PORT[self.home]))
                break
            except OSError:
                time.sleep(0.2)
        s.sendall((json.dumps({'op': 'subscribe', 'token': token}) + '\n').encode())
        for line in s.makefile():
            with self.lock:
                self.got.append((time.time(), json.loads(line)))

    def wait(self, pred, timeout):
        end = time.time() + timeout
        while time.time() < end:
            with self.lock:
                for t, e in self.got:
                    if pred(e):
                        return t, e
            time.sleep(0.02)
        return None, None

    def clear(self):
        with self.lock:
            self.got.clear()


pa, pb = start(A), start(B)
try:
    hello = call(A, op='hello')
    pid = hello['projects'][0]['id']
    keys = sorted(json.load(open(os.path.join(A, 'projects', pid[:8], 'app.json'))).keys())
    ea, eb = Events(A), Events(B)
    ea.start(); eb.start()
    time.sleep(1)
    # only A is told about B; B must learn A from the incoming connection and watch it back
    call(A, op='add-teammate', project=pid, device=BID)
    t, _ = ea.wait(lambda e: e.get('event') == 'teammate' and e.get('online'), 60)
    res['a_sees_b_online'] = t is not None
    t, _ = eb.wait(lambda e: e.get('event') == 'teammate' and e.get('online'), 60)
    res['b_watches_a_back'] = t is not None
    eb.wait(lambda e: e.get('event') == 'synced', 30)  # B's first catch-up (join)

    # 1. A -> B, N edits, time from the app's save on A to the synced event on B
    lat = []
    for i in range(N):
        eb.clear()
        t0 = time.time()
        v = call(A, op='set', project=pid, key=keys[i % len(keys)], field='develop.exposure', value=round(0.1 * (i + 1), 2))['version']
        t, e = eb.wait(lambda e: e.get('event') == 'synced', 10)
        lat.append(round((t - t0) * 1000) if t else None)
    ok = [x for x in lat if x is not None]
    res['a_to_b'] = {'edits': N, 'arrived': len(ok), 'median_ms': sorted(ok)[len(ok) // 2] if ok else None, 'max_ms': max(ok) if ok else None}

    # 2. B -> A
    ea.clear()
    t0 = time.time()
    call(B, op='set', project=pid, key=keys[0], field='develop.contrast', value=25)
    t, e = ea.wait(lambda e: e.get('event') == 'synced', 10)
    res['b_to_a_ms'] = round((t - t0) * 1000) if t else None

    # 3. concurrent: different photos, then the same setting at the same moment
    def both(fa, fb):
        th = [threading.Thread(target=fa), threading.Thread(target=fb)]
        [x.start() for x in th]; [x.join() for x in th]
    both(lambda: call(A, op='set', project=pid, key=keys[1], field='rating', value=4),
         lambda: call(B, op='set', project=pid, key=keys[2], field='rating', value=2))
    time.sleep(4)
    sa, sb = call(A, op='status', project=pid), call(B, op='status', project=pid)
    res['concurrent_different_items'] = {'same_head': sa['head'] == sb['head'],
        'a_sees_b_rating': call(A, op='get', project=pid, key=keys[2], field='rating') == 2,
        'b_sees_a_rating': call(B, op='get', project=pid, key=keys[1], field='rating') == 4}
    both(lambda: call(A, op='set', project=pid, key=keys[3], field='develop.exposure', value=1.0),
         lambda: call(B, op='set', project=pid, key=keys[3], field='develop.exposure', value=-1.0))
    time.sleep(5)
    sa, sb = call(A, op='status', project=pid), call(B, op='status', project=pid)
    va = call(A, op='get', project=pid, key=keys[3], field='develop.exposure')
    vb = call(B, op='get', project=pid, key=keys[3], field='develop.exposure')
    res['concurrent_same_setting'] = {'same_head': sa['head'] == sb['head'], 'a_value': va, 'b_value': vb}

    # 4. B stops; A edits 5 times; B comes back and must catch up by itself
    pb.terminate(); pb.wait()
    for i in range(5):
        call(A, op='set', project=pid, key=keys[4], field='develop.exposure', value=0.5 + i)
    want = call(A, op='status', project=pid)['head']
    pb = start(B)
    end, got = time.time() + 90, None
    while time.time() < end:
        try:
            got = call(B, op='status', project=pid)['head']
            if got == want:
                break
        except Exception:
            pass
        time.sleep(0.5)
    res['b_catches_up_after_restart'] = got == want
finally:
    for p in (pa, pb):
        p.terminate()
print(json.dumps(res, indent=1))
