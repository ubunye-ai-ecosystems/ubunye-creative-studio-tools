"""S6: kopanod with the real Ubunye Film. Two Film windows on this PC, each linked to its own
kopanod (own KOPANO_HOME, own KOPANO_API_PORT, own device identity). Alice (editor) starts a
project from her Film window; Member1 (sound) joins with a copy of the project file whose
footage is NOT on his side (paths point at a folder that does not exist), so his clips start
offline and the footage has to travel.

Checked:
 1. join: Member1's window gets proxies of every clip (made by Film on Alice's side) and plays;
 2. the two windows hold different id blocks; a razor cut on each side at the same moment makes
    new clips with different ids, and both sides end with all of them;
 3. Alice trims and grades a shot WHILE Member1 sets an audio level: both converge, 0 clashes;
 4. Member1 asks for the originals; his media items are relinked to them;
 5. a frame and a 2 s software H.264 export from each window are byte-identical;
 6. data-loss guard: Member1's Film is closed and a fresh window opens the OLD file; it is
    brought to the project (team edits survive on both sides, its own state is kept aside);
    then Alice reopens an old file in her running window: same.
Owner rule: only Thabang's own footage (T:/kopano-lab/owner-footage).

Usage: python s6_film.py <filmcraft.exe> <kopanod.exe> <filmcraft-cli.exe> <project.fcproj> <work dir>
       <alice lab dir> <m1 lab dir> [--shots DIR]"""
import hashlib, json, os, shutil, socket, subprocess, sys, threading, time

FILM, K, CLI, PROJ, W, ALICE, M1 = sys.argv[1:8]
SHOTS = sys.argv[sys.argv.index('--shots') + 1] if '--shots' in sys.argv else W
FP = {'a': 18481, 'b': 18482}
KP = {'a': 47181, 'b': 47182}  # clear of the Photo and Light runs
TPS = 254_016_000_000
res = {}


class Film:
    def __init__(self, port, timeout=120):
        end = time.time() + timeout
        while True:
            try:
                self.s = socket.create_connection(('127.0.0.1', port), timeout=300)
                break
            except OSError:
                if time.time() > end:
                    raise
                time.sleep(0.3)
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

    def items(self):
        return self.cmd('kopano.items')

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def kcall(who, **req):
    tok = open(f'{W}/{who}/api.token').read().strip()
    s = socket.create_connection(('127.0.0.1', KP[who]), timeout=300)
    s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
    r = json.loads(s.makefile().readline())
    s.close()
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


class Events(threading.Thread):
    def __init__(self, who):
        super().__init__(daemon=True)
        self.who, self.got, self.lock = who, [], threading.Lock()

    def run(self):
        tok = open(f'{W}/{self.who}/api.token').read().strip()
        s = socket.create_connection(('127.0.0.1', KP[self.who]))
        s.sendall((json.dumps({'op': 'subscribe', 'token': tok}) + '\n').encode())
        for line in s.makefile():
            with self.lock:
                self.got.append((time.time(), json.loads(line)))

    def all(self, pred):
        with self.lock:
            return [e for _, e in self.got if pred(e)]


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


def start_film(who, tag=''):
    data = f'{W}/film-data-{who}'
    os.makedirs(data, exist_ok=True)
    log = open(f'{W}/film-{who}{tag}.log', 'w')
    return subprocess.Popen([FILM, '--control', str(FP[who]), '--empty', '--no-recover', '--data-dir', data], stdout=log, stderr=log)


def dismiss_link_dialog(f):
    """Film asks where missing footage is when a file opens; the footage is on its way, so Esc."""
    time.sleep(1.5)
    for _ in range(2):
        try:
            f.rpc('ui.key', {'key': 'Escape'})
        except Exception:
            pass
        time.sleep(0.3)


def shot(f, name):
    path = os.path.abspath(os.path.join(SHOTS, name))
    for _ in range(2):  # let decoders catch up on the frame under the playhead
        time.sleep(1.2)
        f.rpc('ui.screenshot', {})
    f.rpc('ui.screenshot', {'path': path})
    return path


def seq_of(f):
    return f.items()['sequences'][0]


def clip_by(f, kind, i=0):
    return sorted([c for c in seq_of(f)['clips'] if c['kind'] == kind], key=lambda c: c['edit']['cut'][0])[i]


def clip_id_set(f):
    return sorted(c['id'] for c in seq_of(f)['clips'])


def find(f, cid):
    return next((c for c in seq_of(f)['clips'] if c['id'] == cid), None)


def exposure(c):
    return ((c or {}).get('grade') or {}).get('fx', {}).get('lumetri', {}).get('params', {}).get('exposure', {}).get('value', {}).get('Float')


def gain(c):
    return ((c or {}).get('sound') or {}).get('gain_db')


def snap(f):
    s = seq_of(f)
    return json.dumps({'clips': sorted(s['clips'], key=lambda c: c['id']), 'tracks': sorted(s['tracks'], key=lambda t: t['id'])}, sort_keys=True)


def file_hash(p):
    return hashlib.blake2b(open(p, 'rb').read(), digest_size=16).hexdigest()


# ---- setup: project files, homes ----
shutil.rmtree(W, ignore_errors=True)
os.makedirs(SHOTS, exist_ok=True)
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
proj_a = f'{W}/alice/cableway.fcproj'
proj_b = f'{W}/member1/cableway.fcproj'
os.makedirs(os.path.dirname(proj_a))
os.makedirs(os.path.dirname(proj_b))
shutil.copy(PROJ, proj_a)
text = open(PROJ, encoding='utf-8').read()
footage_dir = json.loads(text)['project']['items']
paths = [v['kind']['Media']['media']['File']['path'] for v in footage_dir.values() if 'Media' in v.get('kind', {}) and 'File' in v['kind']['Media']['media']]
src_dir = os.path.dirname(paths[0])
missing = f'{W}/member1/footage-not-here'
open(proj_b, 'w', encoding='utf-8').write(text.replace(src_dir, missing))
shutil.copy(proj_a, f'{W}/alice/old.fcproj')
shutil.copy(proj_b, f'{W}/member1/old.fcproj')
res['footage'] = [os.path.basename(p) for p in paths]
res['member1_footage_present'] = any(os.path.exists(p.replace(src_dir, missing)) for p in paths)

procs = {}
env = dict(os.environ, KOPANO_FILM_CLI=CLI)
try:
    for who in 'ab':
        procs['film-' + who] = start_film(who)
        log = open(f'{W}/{who}/kopanod.log', 'w')
        procs['k-' + who] = subprocess.Popen([K, 'run'], stdout=log, stderr=log, env=dict(env, KOPANO_HOME=f'{W}/{who}', KOPANO_API_PORT=str(KP[who])))
    fa, fb = Film(FP['a']), Film(FP['b'])
    fa.cmd('file.open', path=os.path.abspath(proj_a))
    fb.cmd('file.open', path=os.path.abspath(proj_b))
    dismiss_link_dialog(fb)
    wait_for(lambda: os.path.exists(f'{W}/b/api.token'), 30)
    time.sleep(10)  # both services online
    m1 = json.load(open(f'{W}/b/cert.json'))['body']
    adev = json.load(open(f'{W}/a/cert.json'))['body']['device']
    ea, eb = Events('a'), Events('b')
    ea.start(); eb.start()

    v0 = clip_by(fa, 'video')
    a0 = clip_by(fa, 'audio')
    vid, aid = v0['id'], a0['id']
    at = (v0['edit']['cut'][0] + v0['edit']['cut'][1] / 2) / TPS
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)

    # ---- 1. Alice starts the project, Member1 joins ----
    t0 = time.time()
    made = kcall('a', op='create-project', name='Cable way (S6)', mode='live', app={'kind': 'film-control', 'port': FP['a']},
                 members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    res['create'] = {'items': made['items'], 'ms': round((time.time() - t0) * 1000)}
    t0 = time.time()
    kcall('b', op='join', project=pid, **{'from': adev}, app={'kind': 'film-control', 'port': FP['b']})
    res['join_ms'] = round((time.time() - t0) * 1000)
    ms = wait_for(lambda: kcall('b', op='media', project=pid)['from_proxy'] == len(paths), 300, 0.5)
    res['proxies'] = {'all_ms': round((time.time() - t0) * 1000) if ms is not None else None,
                      'mb': sum(e.get('mb_done', 0) for e in eb.all(lambda e: e.get('event') == 'media' and 'file' in e)[-1:]),
                      'events': [e for e in eb.all(lambda e: e.get('event') == 'media' and 'file' in e)],
                      'made_on_alice': ea.all(lambda e: e.get('event') == 'preview-made')}
    res['proxy_files_kb'] = sorted(round(os.path.getsize(os.path.join(dp, f)) / 1000) for dp, _, fs in os.walk(f'{W}/b/media/previews') for f in fs)
    res['media_b_after_join'] = {k: v for k, v in kcall('b', op='media', project=pid).items() if k != 'items'}
    time.sleep(2)
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shot(fa, '1-joined-A.png'); shot(fb, '1-joined-B.png')
    sa, sb = kcall('a', op='media', project=pid), kcall('b', op='media', project=pid)
    res['id_blocks'] = [sa['block'], sb['block']]
    res['id_blocks_in_film'] = [fa.cmd('kopano.idBlock')['block'], fb.cmd('kopano.idBlock')['block']]

    # ---- 2. a razor cut on each side at the same moment: new clips, different ids ----
    before = set(clip_id_set(fa))
    v2 = clip_by(fa, 'video', 1)
    t_cut_a = v2['edit']['cut'][0] + v2['edit']['cut'][1] // 2
    t_cut_b = v0['edit']['cut'][0] + v0['edit']['cut'][1] // 3
    th = [threading.Thread(target=lambda: fa.cmd('timeline.razor', time=t_cut_a, clip=v2['id'])),
          threading.Thread(target=lambda: fb.cmd('timeline.razor', time=t_cut_b, clip=vid))]
    [x.start() for x in th]; [x.join() for x in th]
    ms = wait_for(lambda: len(clip_id_set(fa)) > len(before) and snap(fa) == snap(fb), 30, 0.2)
    time.sleep(1.5)  # both windows have finished applying (a user edit meanwhile may meet a half-applied cut)
    new_ids = sorted(set(clip_id_set(fa)) - before)
    res['razor'] = {'converged_ms': ms, 'clips_before': len(before), 'clips_after': len(clip_id_set(fa)),
                    'new_ids': new_ids, 'new_id_blocks': sorted({i >> 24 for i in new_ids}), 'ids_unique': len(new_ids) == len(set(new_ids))}
    vid = clip_by(fa, 'video')['id']  # the first shot (its head part after Member1's cut)

    # ---- 3. Alice trims and grades WHILE Member1 sets the level ----
    def editor():
        fa.cmd('timeline.trim', clip=vid, edge='in', deltaFrames=15)
        for p, v in (('exposure', 0.9), ('temperature', 45.0), ('saturation', 150.0), ('contrast', 25.0)):
            fa.cmd('effects.setParam', clip=vid, effect='lumetri', param=p, value=v)

    def sound():
        fb.cmd('clip.audioGain', clips=[aid], mode='set', db=-9.0)
    want_cut = None
    t0 = time.time()
    errs = []

    def safe(fn):
        def run():
            try:
                fn()
            except Exception as e:
                errs.append(str(e))
        return run
    th = [threading.Thread(target=safe(editor)), threading.Thread(target=safe(sound))]
    [x.start() for x in th]; [x.join() for x in th]
    want_cut = find(fa, vid)['edit']['cut']
    time.sleep(0.5)
    shot(fa, '2-coediting-A.png'); shot(fb, '2-coediting-B.png')
    t_b = wait_for(lambda: find(fb, vid)['edit']['cut'] == want_cut and abs((exposure(find(fb, vid)) or 0) - 0.9) < 1e-6, 60, 0.1)
    t_a = wait_for(lambda: abs((gain(find(fa, aid)) or 0) + 9.0) < 1e-6, 60, 0.1)
    res['coedit_errors'] = errs
    res['coedit'] = {'alice_trim_grade_in_member1_ms': t_b, 'member1_gain_in_alice_ms': t_a, 'since_start_ms': round((time.time() - t0) * 1000),
                     'clashes': sum(e.get('clashes', 0) for e in ea.all(lambda e: e.get('event') == 'synced') + eb.all(lambda e: e.get('event') == 'synced'))}
    time.sleep(3)
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shot(fa, '3-converged-A.png'); shot(fb, '3-converged-B.png')

    # ---- 4. Member1 asks for the originals ----
    t0 = time.time()
    kcall('b', op='fetch-original', project=pid, key='all')
    ms = wait_for(lambda: kcall('b', op='media', project=pid)['from_original'] == len(paths), 300, 0.5)
    res['originals'] = {'all_ms': ms, 'events': eb.all(lambda e: e.get('event') == 'original' and e.get('state') in ('ready', 'failed'))}
    time.sleep(3)

    # ---- 5. same items, same frame, same export ----
    wait_for(lambda: snap(fa) == snap(fb), 30, 0.3)
    res['identical_timeline'] = snap(fa) == snap(fb)
    if not res['identical_timeline']:
        ca = {c['id']: c for c in seq_of(fa)['clips']}
        cb = {c['id']: c for c in seq_of(fb)['clips']}
        res['timeline_diff'] = [i for i in set(ca) | set(cb) if ca.get(i) != cb.get(i)][:10]
    frames, movies = [], []
    for f, tag in ((fa, 'A'), (fb, 'B')):
        p = os.path.abspath(f'{SHOTS}/frame-{tag}.png')
        f.cmd('file.exportFrame', path=p, time=int(at * TPS), target='program')
        frames.append(file_hash(p))
        p = os.path.abspath(f'{SHOTS}/export-{tag}.mp4')
        t0 = time.time()
        f.cmd('file.exportMedia', path=p, format='h264', hardwareEncoding='off', range='custom', startSeconds=0.0, endSeconds=2.0, wait=True)
        movies.append({'hash': file_hash(p), 'bytes': os.path.getsize(p), 's': round(time.time() - t0, 1)})
    res['frames_identical'] = frames[0] == frames[1]
    res['exports'] = movies
    res['exports_identical'] = movies[0]['hash'] == movies[1]['hash']
    shot(fa, '4-originals-A.png'); shot(fb, '4-originals-B.png')
    head_before = kcall('a', op='status', project=pid)['head']

    # ---- 6a. Member1's Film closes; a fresh window opens the OLD file ----
    fb.close()
    procs['film-b'].terminate(); procs['film-b'].wait()
    procs['film-b'] = start_film('b', '-2')
    fb = Film(FP['b'])
    # the fresh window first runs with nothing open while kopanod links it (the race S6 found)
    time.sleep(8)
    res['empty_window_kept_team_work'] = {'alice_clips': len(clip_id_set(fa)), 'alice_grade': abs((exposure(find(fa, vid)) or 0) - 0.9) < 1e-6}
    fb.cmd('file.open', path=os.path.abspath(f'{W}/member1/old.fcproj'))
    dismiss_link_dialog(fb)
    t0 = time.time()
    ms = wait_for(lambda: find(fb, vid) is not None and find(fb, vid)['edit']['cut'] == want_cut and abs((exposure(find(fb, vid)) or 0) - 0.9) < 1e-6 and abs((gain(find(fb, aid)) or 0) + 9) < 1e-6, 90, 0.3)
    time.sleep(4)
    res['fresh_window_member1'] = {'brought_to_project_ms': ms, 'kept_versions': json.load(open(f'{W}/b/projects/{pid[:8]}/kept.json')) if os.path.exists(f'{W}/b/projects/{pid[:8]}/kept.json') else [],
                                   'alice_still_has_grade': abs((exposure(find(fa, vid)) or 0) - 0.9) < 1e-6, 'alice_still_has_gain': abs((gain(find(fa, aid)) or 0) + 9) < 1e-6,
                                   'alice_clips': len(clip_id_set(fa)), 'member1_clips': len(clip_id_set(fb)), 'same_clips': clip_id_set(fa) == clip_id_set(fb),
                                   'head_unchanged': kcall('a', op='status', project=pid)['head'] == head_before}
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shot(fa, '5-member1-fresh-window-A.png'); shot(fb, '5-member1-fresh-window-B.png')

    # ---- 6b'. Alice starts a new, unrelated project in her running window: left alone ----
    fa.cmd('file.newProject', name='something else')
    time.sleep(8)
    res['other_project_open'] = {'member1_clips': len(clip_id_set(fb)), 'member1_grade': abs((exposure(find(fb, vid)) or 0) - 0.9) < 1e-6,
                                 'alice_window_sequences': len(fa.items()['sequences'])}

    # ---- 6b. Alice reopens the old file in her running window ----
    fa.cmd('file.open', path=os.path.abspath(f'{W}/alice/old.fcproj'))
    ms = wait_for(lambda: find(fa, vid) is not None and abs((exposure(find(fa, vid)) or 0) - 0.9) < 1e-6 and abs((gain(find(fa, aid)) or 0) + 9) < 1e-6 and clip_id_set(fa) == clip_id_set(fb), 60, 0.3)
    time.sleep(4)
    res['reopen_alice'] = {'brought_to_project_ms': ms, 'member1_still_has_grade': abs((exposure(find(fb, vid)) or 0) - 0.9) < 1e-6,
                           'kept_versions': len(json.load(open(f'{W}/a/projects/{pid[:8]}/kept.json'))) if os.path.exists(f'{W}/a/projects/{pid[:8]}/kept.json') else 0,
                           'new_block': fa.cmd('kopano.idBlock')['block']}
    # ---- 7. quiet: both heads settle on one version and stay there (no flip-flop) ----
    wait_for(lambda: kcall('a', op='status', project=pid)['head'] == kcall('b', op='status', project=pid)['head'], 30, 0.5)
    seen = set()
    for _ in range(20):
        seen.add((kcall('a', op='status', project=pid)['head'], kcall('b', op='status', project=pid)['head']))
        time.sleep(1)
    res['quiet_20s'] = {'head_pairs_seen': len(seen), 'stable': len(seen) == 1 and len(next(iter(seen)) and set(next(iter(seen)))) == 1}
    sa, sb = kcall('a', op='status', project=pid), kcall('b', op='status', project=pid)
    res['same_head'] = sa['head'] == sb['head']
    res['identical_timeline_end'] = snap(fa) == snap(fb)
    res['errors_a'] = ea.all(lambda e: 'error' in e)[:5]
    res['errors_b'] = eb.all(lambda e: 'error' in e)[:5]
finally:
    for p in procs.values():
        try:
            p.terminate()
        except Exception:
            pass
    json.dump(res, open(f'{SHOTS}/s6_film.json', 'w'), indent=1)
    print(json.dumps(res, indent=1))
