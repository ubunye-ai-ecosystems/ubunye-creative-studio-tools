"""Shared helpers for the S8 scenarios ("I approve changes" test gaps, 09 section 9).

Each S8 script runs real app windows (Light, Photo or Film), each with its own kopanod (own
KOPANO_HOME and KOPANO_API_PORT, own device identity), prints PASS/FAIL per check as it goes,
and ends with one JSON line: {"scenario", "checks": {name: {ok, detail}}, "passed", "total"}.
Screenshots of every window at every step go to --shots (default T:/kopano-lab/approve-gaps/<name>).
Owner rule: only the owner's own photos and footage."""
import json, os, shutil, socket, subprocess, sys, threading, time


def arg(name, default=None):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


class Run:
    """Results of one scenario: checks, timings, steps (screenshots)."""

    def __init__(self, name, work):
        self.name, self.W = name, os.path.abspath(work)
        self.res = {'scenario': name, 'checks': {}, 'timings': {}, 'steps': []}
        self.shots = os.path.abspath(arg('--shots', f'T:/kopano-lab/approve-gaps/{name}'))
        self.procs = []

    def check(self, name, ok, detail=None):
        self.res['checks'][name] = {'ok': bool(ok), 'detail': detail}
        print(('PASS ' if ok else 'FAIL ') + name + ('' if detail is None else f'  {detail}'), flush=True)
        return bool(ok)

    def time(self, name, ms):
        self.res['timings'][name] = ms

    def finish(self):
        for p in reversed(self.procs):
            try:
                p.terminate()
            except Exception:
                pass
        for p in self.procs:
            try:
                p.wait(10)
            except Exception:
                pass
        c = self.res['checks']
        fails = [k for k, v in c.items() if not v['ok']]
        self.res['passed'], self.res['total'], self.res['failed'] = len(c) - len(fails), len(c), fails
        os.makedirs(self.shots, exist_ok=True)
        json.dump(self.res, open(os.path.join(self.shots, 'result.json'), 'w'), indent=1, default=str)
        print(json.dumps({'scenario': self.name, 'passed': self.res['passed'], 'total': self.res['total'], 'failed': fails,
                          'timings': self.res['timings'], 'checks': c}, default=str))
        return 0 if not fails else 1


def wait_for(fn, timeout, step=0.1):
    t0, end = time.time(), time.time() + timeout
    while time.time() < end:
        try:
            if fn():
                return round((time.time() - t0) * 1000)
        except Exception:
            pass
        time.sleep(step)
    return None


def fresh_homes(W, ids):
    """W/<who> per identity dir in `ids` {who: dir}: device key and certificate only."""
    shutil.rmtree(W, ignore_errors=True)
    for who, src in ids.items():
        os.makedirs(f'{W}/{who}/projects')
        for f in ('device.key', 'cert.json'):
            shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')


class Kopanod:
    """One kopanod with its own home and API port."""

    def __init__(self, run, who, exe, port, env=None):
        self.run, self.who, self.exe, self.port, self.env = run, who, exe, port, env
        self.home = f'{run.W}/{who}'
        self.p = None

    def start(self):
        log = open(f'{self.home}/kopanod.log', 'a', encoding='utf-8')
        env = dict(os.environ, **(self.env or {}), KOPANO_HOME=self.home, KOPANO_API_PORT=str(self.port))
        self.p = subprocess.Popen([self.exe, 'run', '--home', self.home, '--api-port', str(self.port)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)

        def pump(out, log):  # each line with the time it was written (ms), for timings
            for line in iter(out.readline, b''):
                log.write(f'{time.time():.3f} ' + line.decode('utf-8', 'replace'))
                log.flush()
        threading.Thread(target=pump, args=(self.p.stdout, log), daemon=True).start()
        self.run.procs.append(self.p)
        wait_for(lambda: os.path.exists(f'{self.home}/api.token'), 30)
        wait_for(lambda: self.call(op='hello', _tries=1) is not None, 30, 0.3)
        return self

    def stop(self):
        if self.p:
            self.p.terminate()
            self.p.wait(30)
            self.p = None

    def call(self, _tries=100, **req):
        tok = open(f'{self.home}/api.token').read().strip()
        s = None
        for _ in range(_tries):
            try:
                s = socket.create_connection(('127.0.0.1', self.port), timeout=300)
                break
            except OSError:
                time.sleep(0.2)
        if s is None:
            raise RuntimeError(f'kopanod {self.who} not reachable')
        s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
        r = json.loads(s.makefile().readline())
        s.close()
        if not r['ok']:
            raise RuntimeError(r['error'])
        return r['result']

    def log_count(self, text):
        return open(f'{self.home}/kopanod.log', encoding='utf-8', errors='replace').read().count(text)

    def kept(self, pid):
        f = f'{self.home}/projects/{pid[:8]}/kept.json'
        return len(json.load(open(f))) if os.path.exists(f) else 0


def cert(dir_):
    return json.load(open(os.path.join(dir_, 'cert.json')))['body']


# ---- Light ---------------------------------------------------------------------------------------

class Light:
    """One Light window (control channel). `library` None = an in-memory session."""

    def __init__(self, run, who, exe, port, photos, kopanod, library=None):
        self.run, self.who, self.exe, self.port, self.photos_dir, self.k, self.library = run, who, exe, port, photos, kopanod, library
        self.p, self.n, self.s = None, 0, None

    def start(self):
        lib = ['--library', self.library] if self.library else ['--memory']
        env = dict(os.environ, KOPANO_HOME=self.k.home, KOPANO_API_PORT=str(self.k.port))
        self.p = subprocess.Popen([self.exe, *lib, '--control', str(self.port), '--no-demo', self.photos_dir], env=env,
                                  stdout=subprocess.DEVNULL, stderr=open(f'{self.run.W}/light-{self.who}.log', 'a'))
        self.run.procs.append(self.p)
        self.connect()
        n = len(os.listdir(self.photos_dir))
        wait_for(lambda: len(self.photos()) >= n, 120, 1)
        return self

    def connect(self):
        for _ in range(300):
            try:
                self.s = socket.create_connection(('127.0.0.1', self.port), timeout=60)
                self.f = self.s.makefile('r')
                return
            except OSError:
                time.sleep(0.2)
        raise RuntimeError(f'Light {self.who} control port not reachable')

    def quit(self):
        """Close the window the way a person does (app.quit), confirming a prompt if one comes."""
        try:
            self.rpc('app.quit')
        except Exception:
            pass
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            for b in ('button:quitRetry', 'button:quitAnyway'):
                try:
                    self.rpc('ui.clickWidget', {'id': b})
                except Exception:
                    pass
                try:
                    self.p.wait(10)
                    break
                except subprocess.TimeoutExpired:
                    pass
            if self.p.poll() is None:
                self.p.kill()
                self.p.wait()
        try:
            self.s.close()
        except Exception:
            pass

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

    def id_of(self, key_):
        return next(p['id'] for p in self.photos() if key(p) == key_)

    def dev(self, i, ctl):
        group, name = ctl.split('.')
        return self.cmd('photo.inspect', id=i).get('develop', {}).get(group, {}).get(name)

    def rating(self, i):
        return self.cmd('photo.inspect', id=i).get('rating')

    def shot(self, tag):
        p = os.path.abspath(f'{self.run.shots}/{tag}-{self.who}.png')
        os.makedirs(self.run.shots, exist_ok=True)
        try:
            self.rpc('ui.settle', {'timeoutMs': 4000})
        except Exception:
            pass
        try:
            self.rpc('ui.screenshot', {'path': p})
            return p
        except Exception as e:
            self.run.res.setdefault('screenshot_errors', []).append(f'{tag}-{self.who}: {e}')


def key(p):
    return p.get('content_hash') or f"{p.get('fileName') or p.get('file_name')}|{p.get('captured')}"


def shots(run, windows, tag, caption):
    step = {'tag': tag, 'caption': caption}
    for w in windows:
        if w is not None and w.p is not None and w.p.poll() is None:
            step[w.who] = w.shot(tag)
    run.res['steps'].append(step)

