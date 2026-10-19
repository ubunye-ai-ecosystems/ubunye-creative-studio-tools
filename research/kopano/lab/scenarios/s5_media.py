"""Slice "media": photos travel to a teammate who does not have them, in the real Light app.

Two Light windows on this PC, each with its own kopanod and device identity. Alice's Light has
her photos (the owner's own ARWs, copies); Member1's Light starts EMPTY (--memory --no-demo, no
files). Member1 joins the project and must receive Alice's photos as previews, edit them, see
Alice's edits, then fetch one original, which replaces its preview with every edit kept.
Every step is screenshotted in both windows (ui.screenshot over the control channel; no OS
input) and put side by side with a caption.

Usage: python s5_media.py <lightcraft.exe> <lightcraft-cli.exe> <kopanod.exe> <work dir>
       <alice lab dir> <m1 lab dir> <photos dir> [--port-base 47200 --light-base 18200]"""
import json, os, shutil, socket, subprocess, sys, threading, time

LIGHT, CLI, K, W, ALICE, M1, PHOTOS = sys.argv[1:8]
opt = lambda n, d: int(sys.argv[sys.argv.index(n) + 1]) if n in sys.argv else d
KB, LB = opt('--port-base', 47200), opt('--light-base', 18200)
LP, KP = {'a': LB + 1, 'b': LB + 2}, {'a': KB + 1, 'b': KB + 2}
SHOTS = os.path.join(W, 'shots')
res = {'steps': []}


class Light:
    def __init__(self, port):
        for _ in range(300):
            try:
                self.s = socket.create_connection(('127.0.0.1', port), timeout=120)
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
        page = self.cmd('catalog.query', filter={}, offset=0, limit=500)
        return page.get('photos', page) if isinstance(page, dict) else page

    def by_stem(self, stem):
        return next((p for p in self.photos() if str(p.get('fileName', '')).upper().startswith(stem)), None)

    def develop(self, i):
        return self.cmd('photo.inspect', id=i).get('develop', {})


def kcall(who, **req):
    tok = open(f'{W}/{who}/api.token').read().strip()
    s = socket.create_connection(('127.0.0.1', KP[who]), timeout=300)
    s.sendall((json.dumps({**req, 'token': tok}) + '\n').encode())
    r = json.loads(s.makefile().readline())
    s.close()
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


def subscribe(who, sink):
    def run():
        tok = open(f'{W}/{who}/api.token').read().strip()
        s = socket.create_connection(('127.0.0.1', KP[who]))
        s.sendall((json.dumps({'op': 'subscribe', 'token': tok}) + '\n').encode())
        for line in s.makefile():
            try:
                sink.append((time.time(), json.loads(line)))
            except ValueError:
                pass
    threading.Thread(target=run, daemon=True).start()


def wait_for(fn, timeout, step=0.2):
    t0, end = time.time(), time.time() + timeout
    while time.time() < end:
        try:
            if fn():
                return round((time.time() - t0) * 1000)
        except Exception:
            pass
        time.sleep(step)
    return None


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


def side_by_side(step):
    from PIL import Image, ImageDraw, ImageFont
    a, b = Image.open(step['a']).convert('RGB'), Image.open(step['b']).convert('RGB')
    h = max(a.height, b.height)
    scale = 900 / h if h > 900 else 1
    a = a.resize((round(a.width * scale), round(a.height * scale)))
    b = b.resize((round(b.width * scale), round(b.height * scale)))
    try:
        font = ImageFont.truetype('C:/Windows/Fonts/segoeui.ttf', 26)
        small = ImageFont.truetype('C:/Windows/Fonts/segoeui.ttf', 20)
    except OSError:
        font = small = ImageFont.load_default()
    top, gap = 92, 16
    out = Image.new('RGB', (a.width + b.width + gap, max(a.height, b.height) + top), (24, 24, 24))
    out.paste(a, (0, top))
    out.paste(b, (a.width + gap, top))
    d = ImageDraw.Draw(out)
    d.text((14, 8), step['caption'], fill=(255, 214, 0), font=font)
    d.text((14, 54), "Alice (owner, has the originals)", fill=(230, 230, 230), font=small)
    d.text((a.width + gap + 14, 54), "Member1 (started empty)", fill=(230, 230, 230), font=small)
    p = os.path.join(os.path.dirname(os.path.dirname(step['a'])), f"{step['tag']}-side-by-side.png")
    out.save(p)
    step['side_by_side'] = p


shutil.rmtree(W, ignore_errors=True)
os.makedirs(SHOTS)
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
env = {**os.environ, 'KOPANO_LIGHT_CLI': CLI}
procs, events = [], []
try:
    for who in 'ab':
        args = [LIGHT, '--memory', '--control', str(LP[who]), '--no-demo'] + ([PHOTOS] if who == 'a' else [])
        procs.append(subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=open(f'{W}/light-{who}.log', 'w')))
        log = open(f'{W}/{who}/kopanod.log', 'w')
        procs.append(subprocess.Popen([K, 'run', '--home', f'{W}/{who}', '--api-port', str(KP[who])], stdout=log, stderr=log, env=env))
    la, lb = Light(LP['a']), Light(LP['b'])
    n_photos = len([f for f in os.listdir(PHOTOS) if not f.startswith('.')])
    wait_for(lambda: len(la.photos()) >= n_photos, 120, 1)
    res['start'] = {'alice_photos': len(la.photos()), 'member1_photos': len(lb.photos())}
    while not os.path.exists(f'{W}/b/api.token'):
        time.sleep(0.2)
    time.sleep(12)  # both services online
    subscribe('b', events)
    events_a = []
    subscribe('a', events_a)
    for l in (la, lb):
        l.rpc('ui.set', {'view': 'photoGrid'})
    shots('0-start', '0. Alice has her 5 photos. Member1\'s Light is empty.')

    m1 = json.load(open(f'{W}/b/cert.json'))['body']
    adev = json.load(open(f'{W}/a/cert.json'))['body']['device']
    made = kcall('a', op='create-project', name='Media slice shoot', app={'kind': 'light-control', 'port': LP['a']},
                 members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    res['create'] = made
    t0 = time.time()
    kcall('b', op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': LP['b']})
    res['join_ms'] = round((time.time() - t0) * 1000)
    first = wait_for(lambda: len(lb.photos()) >= 1, 240, 0.25)
    every = wait_for(lambda: len(lb.photos()) >= n_photos, 240, 0.25)
    res['previews'] = {'first_photo_ms': first and round(first + res['join_ms']), 'all_photos_ms': every and round((time.time() - t0) * 1000),
                       'member1_photos': len(lb.photos())}
    time.sleep(2)
    st = kcall('b', op='media', project=pid)
    res['previews']['media_status'] = {k: st[k] for k in ('photos', 'from_original', 'from_preview', 'waiting')}
    pv_dir = f'{W}/b/media/previews/{pid[:8]}'
    sizes = {f: os.path.getsize(os.path.join(pv_dir, f)) for f in os.listdir(pv_dir)} if os.path.isdir(pv_dir) else {}
    res['previews']['files'] = sizes
    res['previews']['mb_total'] = round(sum(sizes.values()) / 1e6, 2)
    res['previews']['original_mb_total'] = round(sum(os.path.getsize(os.path.join(PHOTOS, f)) for f in os.listdir(PHOTOS)) / 1e6, 1)
    res['previews']['events'] = [e for _, e in events if e.get('event') in ('media',)][-3:]
    shots('1-previews-arrived', '1. Member1 joined: Alice\'s 5 photos arrived as previews (no originals moved).')

    # the same photo large in both windows
    ta, tb = la.by_stem('DSC08379'), lb.by_stem('DSC08379')
    ida, idb = ta['id'], tb['id']
    res['photo'] = {'alice': ta['fileName'], 'member1': tb['fileName'], 'member1_kind': tb.get('kind')}
    la.cmd('library.select', ids=[ida], active=ida)
    lb.cmd('library.select', ids=[idb], active=idb)
    for l in (la, lb):
        l.rpc('ui.set', {'view': 'detail'})
    time.sleep(2)
    shots('2-same-photo', '2. The same photo in both windows: Member1 sees it from its preview.')

    # Alice edits; Member1's preview must follow
    t0 = time.time()
    la.cmd('develop.set', control='light.exposure', value=1.25, ids=[ida])
    res['a_to_b_exposure_ms'] = wait_for(lambda: abs((lb.develop(idb).get('light', {}).get('exposure') or 0) - 1.25) < 1e-6, 30)
    time.sleep(1.5)
    shots('3-alice-exposure', '3. Alice set exposure +1.25 on her RAW: Member1\'s preview follows.')

    # Member1 edits the preview; Alice's RAW must follow (key = the ORIGINAL's)
    lb.cmd('library.select', ids=[idb], active=idb)
    lb.cmd('develop.set', control='light.contrast', value=40, ids=[idb])
    lb.cmd('photo.rate', rating=4, ids=[idb])
    res['b_to_a_contrast_ms'] = wait_for(lambda: abs((la.develop(ida).get('light', {}).get('contrast') or 0) - 40) < 1e-6, 30)
    res['b_to_a_rating_ms'] = wait_for(lambda: la.cmd('photo.inspect', id=ida).get('rating') == 4, 30)
    time.sleep(1.5)
    shots('4-member1-contrast', '4. Member1 set contrast +40 and 4 stars on the preview: Alice\'s RAW follows.')

    # Member1 asks for the original of this photo
    key = st['items'] and next(i['key'] for i in st['items'] if (i.get('name') or '').upper().startswith('DSC08379'))
    t0 = time.time()
    started = kcall('b', op='fetch-original', project=pid, key=key)
    res['original'] = {'started': started}
    done = wait_for(lambda: any(e.get('event') == 'original' and e.get('state') in ('ready', 'failed') for _, e in events), 600, 0.5)
    fin = next((e for _, e in events if e.get('event') == 'original' and e.get('state') in ('ready', 'failed')), None)
    res['original'].update({'ms': done, 'result': fin, 'progress_events': len([1 for _, e in events if e.get('event') == 'original' and e.get('state') == 'downloading'])})
    wait_for(lambda: (lb.by_stem('DSC08379') or {}).get('fileName', '').upper().endswith('.ARW'), 30)
    tb2 = lb.by_stem('DSC08379')
    d2 = lb.develop(tb2['id'])
    res['original']['member1_now'] = {'file': tb2['fileName'], 'kind': tb2.get('kind'), 'exposure': d2.get('light', {}).get('exposure'),
                                      'contrast': d2.get('light', {}).get('contrast'), 'rating': lb.cmd('photo.inspect', id=tb2['id']).get('rating'),
                                      'photos_in_window': len(lb.photos())}
    idb = tb2['id']
    lb.cmd('library.select', ids=[idb], active=idb)
    lb.rpc('ui.set', {'view': 'detail'})
    time.sleep(2)
    shots('5-original-arrived', '5. Member1 fetched the original: the RAW replaced the preview, every edit kept.')

    # and edits still flow on the original
    la.cmd('develop.set', control='light.exposure', value=0.5, ids=[ida])
    res['after_swap_a_to_b_ms'] = wait_for(lambda: abs((lb.develop(idb).get('light', {}).get('exposure') or 0) - 0.5) < 1e-6, 30)
    time.sleep(1.5)
    shots('6-edit-on-original', '6. Alice changes exposure to +0.5: Member1\'s original follows.')
    sa, sb = kcall('a', op='status', project=pid), kcall('b', op='status', project=pid)
    res['same_head'] = sa['head'] == sb['head']
    res['media_b'] = {k: v for k, v in kcall('b', op='media', project=pid).items() if k != 'items'}
    res['alice_photos_end'] = len(la.photos())
    res['events_b'] = [e for _, e in events if e.get('event') in ('media', 'original')][:40]
    res['previews_made_by_alice'] = [e for _, e in events_a if e.get('event') == 'preview-made']
finally:
    for p in procs:
        p.terminate()
    for st in res['steps']:
        if 'a' in st and 'b' in st:
            try:
                side_by_side(st)
            except Exception as e:
                st['side_by_side_error'] = str(e)
    json.dump(res, open(os.path.join(W, 'result.json'), 'w'), indent=1)
    print(json.dumps({k: v for k, v in res.items() if k not in ('events_b',)}, indent=1))
