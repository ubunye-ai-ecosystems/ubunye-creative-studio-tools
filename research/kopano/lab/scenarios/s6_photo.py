"""S6: kopanod with the real Ubunye Photo app. Two Photo windows on this PC, each with its own
kopanod (own home, own API port, own device identity). Alice starts a project from her window
(the owner's photo DSC08379 as a .pcraft document); Member1's window starts on a blank document
and joins: it must become Alice's document, with every tile moved as media through kopanod's
membership-checked transfer. Then edits made IN Photo travel both ways with no command:
an Exposure layer, a new layer, brush strokes (only touched tiles move), strokes at the same time
on different tiles (no clash), a stroke made while a change comes in (Photo refuses the stale
update; kopanod saves the stroke and tries again), and finally Member1's Photo is closed and
opened again on the untouched original: the team's edits must come back, nothing is wiped.

Windows are driven only through Photo's control channel (the same commands its own UI runs).
Usage: python s6_photo.py <photocraft.exe> <kopanod.exe> <work dir> <alice lab dir> <m1 lab dir>
         --doc <owner .pcraft> [--photo-base 18400] [--port-base 47130]
         [--kopanod-a <exe>] [--kopanod-b <exe>]  (an older kopanod on one side: compatibility)
         [--raw]  (KOPANO_PHOTO_RAW_TILES=1 on both: tiles travel raw, for before/after)
MB moved is what the receiving kopanod logs per tile fetch (payload plus verification data)."""
import json, os, shutil, socket, subprocess, sys, time

PHOTO, K, W, ALICE, M1 = sys.argv[1:6]
opt = lambda n, d: sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d
DOC = opt('--doc', None)
PB, KB = int(opt('--photo-base', 18400)), int(opt('--port-base', 47130))
PP, KP = {'a': PB + 1, 'b': PB + 2}, {'a': KB + 1, 'b': KB + 2}
KEXE = {'a': opt('--kopanod-a', K), 'b': opt('--kopanod-b', K)}
RAW = '--raw' in sys.argv
res = {'kopanod': KEXE, 'raw_tiles': RAW}
W = os.path.abspath(W)


class Photo:
    """One Photo window and a control connection of its own (kopanod holds another)."""

    def __init__(self, who, open_doc):
        self.who, self.root = who, f'{W}/photo-{who}'
        os.makedirs(self.root, exist_ok=True)
        self.token = f'{W}/photo-{who}.token'
        if os.path.exists(self.token):
            os.remove(self.token)
        self.p = subprocess.Popen([PHOTO, '--control', str(PP[who]), '--control-token-file', self.token,
                                   '--automation-read-root', self.root, '--automation-write-root', self.root],
                                  stdout=subprocess.DEVNULL, stderr=open(f'{W}/photo-{who}.log', 'a'))
        self.n = 0
        for _ in range(450):
            try:
                if os.path.exists(self.token):
                    self.connect()
                    break
            except OSError:
                pass
            time.sleep(0.2)
        self.rpc('ui.resize', {'width': 1200, 'height': 780})
        if open_doc:
            shutil.copy(DOC, f'{self.root}/owner.pcraft')
            self.rpc('app.open', {'path': 'owner.pcraft'})
        else:
            self.cmd('file.new', width=800, height=600, background='white', name='Blank')
        end = time.time() + 90
        while True:
            try:
                self.export()
                break
            except RuntimeError:
                if time.time() > end:
                    raise
                time.sleep(0.3)

    def connect(self):
        # Photo closes a control connection that is quiet for 30 s: open it again when needed
        self.s = socket.create_connection(('127.0.0.1', PP[self.who]), timeout=120)
        self.f, self.last = self.s.makefile('r'), time.time()
        self.rpc('auth', {'token': open(self.token).read().strip()})

    def rpc(self, method, params=None):
        if method != 'auth' and time.time() - self.last > 20:
            self.s.close()
            self.connect()
        self.last = time.time()
        self.n += 1
        self.s.sendall((json.dumps({'id': self.n, 'method': method, 'params': params or {}}) + '\n').encode())
        while True:
            r = json.loads(self.f.readline())
            if r.get('id') == self.n:
                break
        if not r.get('ok'):
            raise RuntimeError(f'{method}: {r.get("error")}')
        return r.get('result')

    def cmd(self, command, **params):
        return self.rpc('engine.execute', {'command': command, 'params': params})

    def export(self):
        return self.cmd('kopano.export')['manifest']['document']

    def quit(self):
        try:
            self.rpc('app.quit')
        except Exception:
            pass
        try:
            self.p.wait(15)
        except subprocess.TimeoutExpired:
            self.p.kill()

    def shot(self, tag):
        try:
            self.rpc('ui.set', {'fit': True})
            time.sleep(0.8)
            self.rpc('ui.screenshot', {'path': f'{tag}.png'})
            os.makedirs(f'{W}/shots', exist_ok=True)
            shutil.copy(f'{self.root}/{tag}.png', f'{W}/shots/{tag}-{self.who}.png')
        except Exception as e:
            res.setdefault('screenshot_errors', []).append(f'{self.who} {tag}: {e}')


def walk_layers(layers, out):
    for l in layers:
        out.append(l)
        walk_layers(l.get('content', {}).get('children', []) or [], out)
    return out


def layers(doc):
    return walk_layers(doc.get('layers', []), [])


def tiles(v, out=None):
    """Every (tile hash) in a document, wherever its surfaces are."""
    out = [] if out is None else out
    if isinstance(v, dict):
        if 'tx' in v and 'hash' in v:
            out.append(v['hash'])
        else:
            for x in v.values():
                tiles(x, out)
    elif isinstance(v, list):
        for x in v:
            tiles(x, out)
    return out


def shape(doc):
    """What must match between two windows: layer names/kinds/settings in order, and every tile."""
    ls = []
    for l in layers(doc):
        c = dict(l.get('content', {}))
        c.pop('children', None)
        ls.append((l.get('name'), json.dumps(c.get('adjustment'), sort_keys=True), l.get('opacity'), l.get('visible')))
    return ls, sorted(tiles(doc.get('layers', []))), json.dumps(doc.get('size'))


def same(a, b):
    return shape(a.export()) == shape(b.export())


def layer_id(w, name):
    return next(l['id'] for l in reversed(layers(w.export())) if l.get('name') == name)


def kcall(who, **req):
    tok = open(f'{W}/{who}/api.token').read().strip()
    s = socket.create_connection(('127.0.0.1', KP[who]), timeout=180)
    s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
    r = json.loads(s.makefile().readline())
    s.close()
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


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


def clashes(who):
    n = 0
    for l in open(f'{W}/{who}/kopanod.log', encoding='utf-8', errors='replace'):
        if '"event":"synced"' in l:
            try:
                n += json.loads(l.split('[kopanod] ', 1)[1]).get('clashes', 0)
            except Exception:
                pass
    return n


def binding(who):
    return {'kind': 'photo-control', 'port': PP[who], 'token_file': f'{W}/photo-{who}.token'}


def moved_kb(who):
    """KB this kopanod received for tiles so far (its 'KB moved' log lines)."""
    n = 0
    for l in open(f'{W}/{who}/kopanod.log', encoding='utf-8', errors='replace'):
        if ' KB moved for ' in l:
            try:
                n += int(l.split('): ', 1)[1].split(' KB moved', 1)[0])
            except Exception:
                pass
    return n


class Moved:
    """MB both kopanods received during a step."""

    def __enter__(self):
        self.k = {w: moved_kb(w) for w in 'ab'}
        return self

    def mb(self, wait=1.0):
        time.sleep(wait)  # the last fetch's log line
        return {f'{w}_received_mb': round((moved_kb(w) - self.k[w]) / 1000, 2) for w in 'ab'}

    def __exit__(self, *a):
        return False


def log_count(who, text):
    return open(f'{W}/{who}/kopanod.log', encoding='utf-8', errors='replace').read().count(text)


shutil.rmtree(W, ignore_errors=True)
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
procs, wins = [], {}
try:
    for who in 'ab':
        log = open(f'{W}/{who}/kopanod.log', 'w')
        env = dict(os.environ, KOPANO_PHOTO_RAW_TILES='1') if RAW else None
        procs.append(subprocess.Popen([KEXE[who], 'run', '--home', f'{W}/{who}', '--api-port', str(KP[who])], stdout=log, stderr=log, env=env))
    wa, wb = Photo('a', True), Photo('b', False)
    wins = {'a': wa, 'b': wb}
    res['start'] = {'alice': shape(wa.export())[2], 'alice_tiles': len(shape(wa.export())[1]), 'member1': shape(wb.export())[2], 'member1_tiles': len(shape(wb.export())[1])}
    while not os.path.exists(f'{W}/b/api.token'):
        time.sleep(0.2)
    time.sleep(12)  # both services online
    m1 = json.load(open(f'{W}/b/cert.json'))['body']
    adev = json.load(open(f'{W}/a/cert.json'))['body']['device']
    wa.shot('0-start'); wb.shot('0-start')

    # 0. Alice starts the project from her window; Member1's blank window joins and must become it
    t0 = time.time()
    made = kcall('a', op='create-project', name='Owner photo (S6)', app=binding('a'),
                 members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    res['create'] = {'items': made['items'], 'ms': round((time.time() - t0) * 1000)}
    t0 = time.time()
    mv = Moved().__enter__()
    kcall('b', op='join', project=pid, **{'from': adev}, app=binding('b'))
    ms = wait_for(lambda: same(wa, wb), 120)
    res['join'] = {'window_became_alices_document_ms': ms, 'total_ms': round((time.time() - t0) * 1000), **mv.mb(),
                   'tiles_moved': len(shape(wb.export())[1]), 'kopanod_b_tile_fetch_log': [l.strip() for l in open(f'{W}/b/kopanod.log', errors='replace') if 'fetched from teammates' in l][:3]}
    time.sleep(1.5)
    wa.shot('1-joined'); wb.shot('1-joined')

    # 1. Alice adds an Exposure adjustment layer, then moves its slider
    wa.cmd('layer.newAdjustmentLayer.exposure', exposure=0.3, offset=0.0, gamma=1.0)
    res['a_to_b_new_exposure_layer_ms'] = wait_for(lambda: same(wa, wb), 30)
    wa.cmd('layer.setAdjustment', exposure=0.7)
    res['a_to_b_exposure_slider_ms'] = wait_for(lambda: same(wa, wb), 30)

    # 2. Alice adds a Retouch layer and paints one brush stroke on it
    wa.cmd('layer.new.layer', name='Retouch')
    res['a_to_b_new_layer_ms'] = wait_for(lambda: same(wa, wb), 30)
    before = set(shape(wa.export())[1])
    mv = Moved().__enter__()
    wa.cmd('paint.stroke', points=[[400, 1880, 1], [800, 1930, 1], [1200, 1890, 1]], size=90, color='#ffd23f')
    new_tiles = len(set(shape(wa.export())[1]) - before)
    res['a_to_b_brush_stroke'] = {'ms': wait_for(lambda: same(wa, wb), 30), 'tiles_changed': new_tiles, 'tiles_in_document': len(shape(wa.export())[1]), **mv.mb()}
    time.sleep(1.5)
    wa.shot('2-alice-brush-stroke'); wb.shot('2-alice-brush-stroke')

    # 3. Member1 paints on the same layer in his window; it must show in Alice's
    wb.cmd('layer.select', layer=layer_id(wb, 'Retouch'))
    before = set(shape(wb.export())[1])
    mv = Moved().__enter__()
    wb.cmd('paint.stroke', points=[[1500, 600, 1], [2100, 700, 1], [2600, 650, 1]], size=80, color='#ff3f9a')
    new_tiles = len(set(shape(wb.export())[1]) - before)
    res['b_to_a_brush_stroke'] = {'ms': wait_for(lambda: same(wa, wb), 30), 'tiles_changed': new_tiles, **mv.mb()}
    time.sleep(1.5)
    wa.shot('3-member1-brush-stroke'); wb.shot('3-member1-brush-stroke')

    # 4. both paint at the same moment, different tiles of the same layer: no clash, both kept
    wa.cmd('layer.select', layer=layer_id(wa, 'Retouch'))
    wa.cmd('paint.stroke', points=[[150, 300, 1], [500, 380, 1]], size=70, color='#3fa7ff')
    wb.cmd('paint.stroke', points=[[2400, 1500, 1], [2800, 1580, 1]], size=70, color='#3fff7a')
    ms = wait_for(lambda: same(wa, wb), 40)
    res['both_paint_at_once'] = {'settled_ms': ms, 'clashes': clashes('a') + clashes('b')}
    time.sleep(1.5)
    wa.shot('4-both-painted'); wb.shot('4-both-painted')

    # 5. Member1 keeps painting while Alice's change comes in: Photo refuses stale updates and
    #    kopanod must save Member1's strokes and try again; nothing may be lost on either side
    retries0 = log_count('b', 'trying again')
    adj_id = next(l['id'] for l in layers(wa.export()) if l.get('content', {}).get('kind') == 'adjustment')
    strokes = 0
    t_end = time.time() + 5
    alice_changes = [-0.1, -0.2, -0.3, -0.4]
    next_change = time.time()
    while time.time() < t_end:
        if alice_changes and time.time() >= next_change:
            wa.cmd('layer.setAdjustment', layer=adj_id, exposure=alice_changes.pop(0))
            next_change = time.time() + 1.1
        y = 900 + strokes * 9
        wb.cmd('paint.stroke', points=[[300, y, 1], [2700, y + 40, 1]], size=10, color='#ffffff')
        strokes += 1
    painted = set(shape(wb.export())[1])  # Member1's tiles right after his last stroke
    ms = wait_for(lambda: same(wa, wb), 40)
    final = set(shape(wa.export())[1])
    adj = [json.loads(x[1]) for x in shape(wb.export())[0] if x[1] != 'null']
    res['edit_while_changes_come_in'] = {'member1_strokes': strokes, 'settled_ms': ms, 'retries_in_kopanod_b': log_count('b', 'trying again') - retries0,
                                         'retries_in_kopanod_a': log_count('a', 'trying again'), 'exposure_in_member1_window': adj,
                                         'member1_tiles_lost': len(painted - final)}
    time.sleep(5)
    later = set(shape(wa.export())[1]) | set(shape(wb.export())[1])
    res['edit_while_changes_come_in']['member1_tiles_lost_5s_later'] = len(painted - later)
    if painted - final:
        json.dump({'painted': sorted(painted), 'final_a': sorted(final), 'b_now': sorted(shape(wb.export())[1])}, open(f'{W}/lost-tiles.json', 'w'))
    time.sleep(1.5)
    wa.shot('5-edit-while-changes-come-in'); wb.shot('5-edit-while-changes-come-in')

    # 6. Member1's Photo closes; it opens again on the untouched original and is attached again.
    #    The team's edits must come back into it, and Alice must keep them (the S3b data loss).
    team = shape(wa.export())
    head_before = kcall('a', op='status', project=pid)['head']
    wb.quit()
    wb = Photo('b', True)
    wins['b'] = wb
    res['fresh_window_tiles_before_attach'] = len(shape(wb.export())[1])
    t0 = time.time()
    kcall('b', op='attach', project=pid, app=binding('b'))
    ms = wait_for(lambda: shape(wb.export()) == team, 60)
    time.sleep(3)  # a few save rounds: nothing may flow back from the fresh window
    res['reopened_window'] = {'team_edits_back_ms': ms, 'alice_still_has_team_edits': shape(wa.export()) == team,
                              'kept_versions_on_b': len(json.load(open(f'{W}/b/projects/{pid[:8]}/kept.json'))) if os.path.exists(f'{W}/b/projects/{pid[:8]}/kept.json') else 0,
                              'alice_head_moved': kcall('a', op='status', project=pid)['head'] != head_before}
    wa.shot('6-reopened'); wb.shot('6-reopened')

    # 7. and the reopened window still co-edits
    wb.cmd('layer.select', layer=layer_id(wb, 'Retouch'))
    wb.cmd('paint.stroke', points=[[2000, 200, 1], [2900, 260, 1]], size=60, color='#c03020')
    res['b_to_a_after_reopen_ms'] = wait_for(lambda: same(wa, wb), 30)
    time.sleep(1.5)
    wa.shot('7-after-reopen-stroke'); wb.shot('7-after-reopen-stroke')

    res['status_a'] = kcall('a', op='status', project=pid)
    res['status_b'] = kcall('b', op='status', project=pid)
    res['same_head'] = res['status_a']['head'] == res['status_b']['head']
    res['windows_match'] = same(wa, wb)
    res['clashes_total'] = clashes('a') + clashes('b')
    res['media_refusals_logged'] = {w: log_count(w, 'media: refused') for w in 'ab'}
    res['packed_tiles_not_usable'] = {w: log_count(w, 'not usable, fetching it raw') for w in 'ab'}
    res['received_mb_total'] = {w: round(moved_kb(w) / 1000, 2) for w in 'ab'}
    res['photo_no_object_retries'] = {w: log_count(w, 'Photo has no object') for w in 'ab'}
finally:
    for w in wins.values():
        w.quit()
    for p in procs:
        p.terminate()
    out = json.dumps(res, indent=1, default=str)
    print(out)
    open(f'{W}/result.json', 'w').write(out)
