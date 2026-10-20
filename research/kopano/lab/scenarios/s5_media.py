"""Slice "media": photos travel to a teammate who does not have them, in the real Light app.

Two Light windows on this PC, each with its own kopanod and device identity. Alice's Light has
her photos (the owner's own ARWs, copies); Member1's Light starts EMPTY (--memory --no-demo, no
files). Member1 joins the project and must receive Alice's photos as previews, edit them, see
Alice's edits, then fetch one original, which replaces its preview with every edit kept.
Every step is screenshotted in both windows (ui.screenshot over the control channel; no OS
input) and put side by side with a caption.

Previews are Light's own Smart Previews (.lcsp) when Alice's Light can export them; with
--owner-light <older lightcraft.exe> Alice runs a Light without that command and kopanod falls
back to rendered JPEGs. Accuracy: the same edit (exposure +2, highlights -60) made by Alice
reaches Member1's preview; both windows render the photo at 1280 px (ui.render) and the mean
absolute difference per channel (out of 255) is reported, as shot and edited.

Usage: python s5_media.py <lightcraft.exe> <lightcraft-cli.exe> <kopanod.exe> <work dir>
       <alice lab dir> <m1 lab dir> <photos dir> [--port-base 47200 --light-base 18200]
       [--owner-light <lightcraft.exe>] [--peer <peer.exe> --stranger <lab dir>]"""
import json, os, shutil, socket, subprocess, sys, threading, time

LIGHT, CLI, K, W, ALICE, M1, PHOTOS = sys.argv[1:8]
opt = lambda n, d: int(sys.argv[sys.argv.index(n) + 1]) if n in sys.argv else d
KB, LB = opt('--port-base', 47200), opt('--light-base', 18200)
LP, KP = {'a': LB + 1, 'b': LB + 2}, {'a': KB + 1, 'b': KB + 2}
OWNER_LIGHT = sys.argv[sys.argv.index('--owner-light') + 1] if '--owner-light' in sys.argv else LIGHT
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


def render(l, i, tag):
    p = os.path.abspath(f'{W}/renders/{tag}.png')
    l.rpc('ui.render', {'id': i, 'size': 1280, 'path': p})
    return p


def diff(pa, pb):
    """Mean absolute difference per channel out of 255 (pixels, and 16 px blocks)."""
    import numpy as np
    from PIL import Image
    a = np.asarray(Image.open(pa).convert('RGB'), dtype=np.float32)
    bi = Image.open(pb).convert('RGB')
    if bi.size != (a.shape[1], a.shape[0]):
        bi = bi.resize((a.shape[1], a.shape[0]), Image.LANCZOS)
    b = np.asarray(bi, dtype=np.float32)
    k = 16
    H, Wd = a.shape[0] // k * k, a.shape[1] // k * k
    blk = lambda x: x[:H, :Wd].reshape(H // k, k, Wd // k, k, 3).mean(axis=(1, 3))
    return {'pixel': round(float(np.abs(a - b).mean()), 3), 'block16': round(float(np.abs(blk(a) - blk(b)).mean()), 3), 'shape': list(a.shape[:2])}


def accuracy(tag, pairs):
    rows = []
    for stem, ia, ib in pairs:
        rows.append({'photo': stem, **diff(render(la, ia, f'{tag}-{stem}-a'), render(lb, ib, f'{tag}-{stem}-b'))})
    px = [r['pixel'] for r in rows]
    return {'mean': round(sum(px) / len(px), 3), 'worst': max(px), 'block16_mean': round(sum(r['block16'] for r in rows) / len(rows), 3), 'rows': rows}


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
os.makedirs(f'{W}/renders')
for who, src in (('a', ALICE), ('b', M1)):
    os.makedirs(f'{W}/{who}/projects')
    for f in ('device.key', 'cert.json'):
        shutil.copy(os.path.join(src, f), f'{W}/{who}/{f}')
env = {**os.environ, 'KOPANO_LIGHT_CLI': CLI}
procs, events = [], []
try:
    for who in 'ab':
        args = [OWNER_LIGHT if who == 'a' else LIGHT, '--memory', '--control', str(LP[who]), '--no-demo'] + ([PHOTOS] if who == 'a' else [])
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
    sizes = {f: os.path.getsize(os.path.join(pv_dir, f)) for f in os.listdir(pv_dir) if not f.startswith('.')} if os.path.isdir(pv_dir) else {}
    res['previews']['formats'] = sorted({os.path.splitext(f)[1] for f in sizes})
    res['previews']['received_mb'] = round(sum(e.get('mb') or 0 for _, e in events if e.get('event') == 'media' and 'mb' in e), 2)
    res['previews']['files'] = sizes
    res['previews']['mb_total'] = round(sum(sizes.values()) / 1e6, 2)
    res['previews']['original_mb_total'] = round(sum(os.path.getsize(os.path.join(PHOTOS, f)) for f in os.listdir(PHOTOS)) / 1e6, 1)
    res['previews']['events'] = [e for _, e in events if e.get('event') in ('media',)][-3:]
    shots('1-previews-arrived', '1. Member1 joined: Alice\'s 5 photos arrived as previews (no originals moved).')

    # accuracy: every photo, as shot, then Alice's edit (exposure +2, highlights -60) on Member1's preview
    stems = sorted(os.path.splitext(f)[0] for f in os.listdir(PHOTOS) if not f.startswith('.'))
    pairs = [(st_, la.by_stem(st_)['id'], lb.by_stem(st_)['id']) for st_ in stems]
    res['accuracy'] = {'as_shot': accuracy('as-shot', pairs)}
    t0 = time.time()
    for _, ia, _ in pairs:
        la.cmd('develop.set', control='light.exposure', value=2.0, ids=[ia])
        la.cmd('develop.set', control='light.highlights', value=-60, ids=[ia])
    ok = lambda ib: (lambda d: abs((d.get('exposure') or 0) - 2.0) < 1e-6 and abs((d.get('highlights') or 0) + 60) < 1e-6)(lb.develop(ib).get('light', {}))
    res['accuracy']['edit_reached_member1_ms'] = wait_for(lambda: all(ok(ib) for _, _, ib in pairs), 60)
    time.sleep(1)
    res['accuracy']['exp+2_hl-60'] = accuracy('exp2-hl60', pairs)
    for _, ia, _ in pairs:
        la.cmd('develop.set', control='light.exposure', value=0.0, ids=[ia])
        la.cmd('develop.set', control='light.highlights', value=0, ids=[ia])
    wait_for(lambda: all(abs((lb.develop(ib).get('light', {}).get('highlights') or 0)) < 1e-6 for _, _, ib in pairs), 60)

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
    src = lb.cmd('photo.inspect', id=tb2['id']).get('source', {})
    d2 = lb.develop(tb2['id'])
    res['original']['member1_now'] = {'file': tb2['fileName'], 'kind': tb2.get('kind'), 'exposure': d2.get('light', {}).get('exposure'),
                                      'contrast': d2.get('light', {}).get('contrast'), 'rating': lb.cmd('photo.inspect', id=tb2['id']).get('rating'),
                                      'photos_in_window': len(lb.photos()), 'source': src.get('path'),
                                      'same_light_id': tb2['id'] == idb}
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
    # the gate: a device with a valid certificate from the same sign-in provider, but not a
    # member of this project, asks Alice's kopanod for an original by its hash
    if '--peer' in sys.argv and '--stranger' in sys.argv:
      try:
        peer, stranger = sys.argv[sys.argv.index('--peer') + 1], sys.argv[sys.argv.index('--stranger') + 1]
        sd = f'{W}/stranger'
        os.makedirs(sd)
        for f in ('device.key', 'cert.json', 'project', 'kopano.redb', 'app.json'):
            shutil.copy(os.path.join(stranger, f), sd)
        json.dump({'id': adev, 'addrs': []}, open(f'{W}/alice-addr.json', 'w'))
        other = next(i for i in st['items'] if not (i.get('name') or '').upper().startswith('DSC08379'))
        orig_hash = next(e for e in json.load(open(f'{W}/b/projects/{pid[:8]}/media.json')).values() if e['name'] == other['name'])['original']['hash']
        r = subprocess.run([peer, 'fetch-media', '--dir', sd, '--from', f'{W}/alice-addr.json', '--hash', orig_hash], capture_output=True, text=True, timeout=120)
        out = (r.stdout + r.stderr).strip().splitlines()
        res['gate'] = {'stranger': json.load(open(f'{sd}/cert.json'))['body']['name'], 'asked_for': other['name'], 'exit': r.returncode,
                       'got_file': r.returncode == 0, 'last_lines': out[-2:]}
        # the client only sees the connection drop; Alice's kopanod says why
        time.sleep(1)
        res['gate']['alice_log'] = [l.strip() for l in open(f'{W}/a/kopanod.log', errors='replace') if 'media: refused' in l][-2:]
      except Exception as e:
        res['gate'] = {'error': repr(e)}
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
