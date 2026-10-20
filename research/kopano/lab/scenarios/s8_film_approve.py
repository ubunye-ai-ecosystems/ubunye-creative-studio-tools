"""S8 Film approve: Ubunye Film in an "I approve changes" project (09 section 9, case 2).

Two Film windows, each with its own kopanod (own KOPANO_HOME and KOPANO_API_PORT, own device);
the owner's 3 Sony clips (T:/kopano-lab/owner-footage). Member1's project file points at footage
that is not on his side, so it travels (proxies, then originals). Checked, screenshots per step:
  1. id blocks: the two windows hold different blocks; a razor cut on each side at the same
     moment gives clips with different ids (his on his branch, hers on main)
  2. Member1 sets an audio level of -9 dB and trims the first shot WHILE Alice grades it on main:
     his window has his trim and level at once, Alice's main and window do not take them, her
     review lists his change, he gets her grade
  3. Alice approves: her window takes his trim, level and razor cut; one main on both
  4. same timeline, same frame and byte-identical 2 s software H.264 export from both windows
  5. Member1 sets -3 dB; Alice declines: her main and window keep -9 dB, he keeps -3 dB
  6. Alice grades again: it reaches him, his declined level stays, nothing offered again
  7. all clip ids unique on both sides
  8. Member1 sets a level while his kopanod is stopped: it stays in his window and reaches review
Usage: python s8_film_approve.py <filmcraft.exe> <kopanod.exe> <filmcraft-cli.exe> <project.fcproj> <work dir>
       <alice id> <m1 id> [--shots DIR] [--film-base 18600] [--port-base 47600]"""
import json, os, shutil, sys, threading, time
from s8_common import Run, Kopanod, Film, TPS, arg, cert, exposure, file_hash, fresh_homes, gain, shots, wait_for

FILM, K, CLI, PROJ, W, ALICE, M1 = sys.argv[1:8]
FB, KB = int(arg('--film-base', 18600)), int(arg('--port-base', 47600))
run = Run('s8_film_approve', W)
fresh_homes(run.W, {'a': ALICE, 'b': M1})
proj_a, proj_b = f'{run.W}/alice/cableway.fcproj', f'{run.W}/member1/cableway.fcproj'
os.makedirs(os.path.dirname(proj_a))
os.makedirs(os.path.dirname(proj_b))
shutil.copy(PROJ, proj_a)
text = open(PROJ, encoding='utf-8').read()
items = json.loads(text)['project']['items']
paths = [v['kind']['Media']['media']['File']['path'] for v in items.values() if 'Media' in v.get('kind', {}) and 'File' in v['kind']['Media']['media']]
src_dir = os.path.dirname(paths[0])
open(proj_b, 'w', encoding='utf-8').write(text.replace(src_dir, run.W.replace(chr(92), '/') + '/member1/footage-not-here'))
run.res['footage'] = [os.path.basename(p) for p in paths]


def both(fa, fb):
    errs = []

    def safe(f):
        def go():
            try:
                f()
            except Exception as e:
                errs.append(str(e))
        return go
    th = [threading.Thread(target=safe(fa)), threading.Thread(target=safe(fb))]
    [t.start() for t in th]; [t.join() for t in th]
    return errs


try:
    env = {'KOPANO_FILM_CLI': CLI}
    ka, kb = Kopanod(run, 'a', K, KB + 1, env).start(), Kopanod(run, 'b', K, KB + 2, env).start()
    fa, fb = Film(run, 'a', FILM, FB + 1).start(), Film(run, 'b', FILM, FB + 2).start()
    fa.cmd('file.open', path=os.path.abspath(proj_a))
    fb.cmd('file.open', path=os.path.abspath(proj_b))
    fb.dismiss_link_dialog()
    time.sleep(10)
    m1, adev = cert(M1), cert(ALICE)['device']
    v0 = fa.clip('video')
    vid, aid = v0['id'], fa.clip('audio')['id']
    at = (v0['edit']['cut'][0] + v0['edit']['cut'][1] / 2) / TPS
    made = ka.call(op='create-project', name='Cable way (approve)', mode='approve', app={'kind': 'film-control', 'port': fa.port},
                   members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    run.check('created in approve mode', made.get('mode') == 'approve', {'items': made['items']})
    kb.call(op='join', project=pid, **{'from': adev}, app={'kind': 'film-control', 'port': fb.port})
    st = lambda k: k.call(op='status', project=pid)
    rv = lambda k: st(k)['review']
    pend = lambda: [p for p in rv(ka)['pending'] if p['account'] == m1['account']]
    ms = wait_for(lambda: kb.call(op='media', project=pid)['from_proxy'] == len(paths), 300, 0.5)
    run.check('member plays every clip from proxies', ms is not None, f'{ms} ms')
    run.check('member window shows main', wait_for(lambda: st(kb)['head'] == rv(ka)['main'] and fa.snap() == fb.snap(), 60) is not None)
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shots(run, (fa, fb), '0-joined', 'Member1 joined Alice\'s "I approve changes" film: his clips play from proxies')
    main0 = rv(ka)['main']

    # 1. id blocks, a razor cut on each side at once
    blocks = [ka.call(op='media', project=pid)['block'], kb.call(op='media', project=pid)['block']]
    in_film = [fa.cmd('kopano.idBlock')['block'], fb.cmd('kopano.idBlock')['block']]
    run.check('1 the two windows hold different id blocks', blocks[0] != blocks[1] and in_film == blocks, {'service': blocks, 'film': in_film})
    before = set(fa.ids())
    v2 = fa.clip('video', 1)
    t_a = v2['edit']['cut'][0] + v2['edit']['cut'][1] // 2
    t_b = v0['edit']['cut'][0] + v0['edit']['cut'][1] // 3
    errs = both(lambda: fa.cmd('timeline.razor', time=t_a, clip=v2['id']), lambda: fb.cmd('timeline.razor', time=t_b, clip=vid))
    time.sleep(2)
    new_a, new_b = set(fa.ids()) - before, set(fb.ids()) - before
    his_new = new_b - new_a
    run.check('1 razor on each side: new ids from each side\'s block', new_a and his_new and {i >> 24 for i in new_a} == {blocks[0]} and {i >> 24 for i in his_new} == {blocks[1]} and not errs,
              {'alice_new': sorted(new_a), 'member_new': sorted(his_new), 'errors': errs})
    run.check('1 his cut waits for review; hers is on main', wait_for(lambda: pend() and rv(ka)['main'] != main0, 60) is not None)
    # his window takes her cut in before he edits on (an edit while a merge is half applied can be
    # refused by Film for an overlap: 09 section 8, open)
    run.check('1 her cut reaches his window', wait_for(lambda: new_a <= set(fb.ids()), 60) is not None)
    time.sleep(2)
    vid = fb.clip('video')['id']  # the first shot's head part after his cut

    # 2. Member1: level -9 dB and a trim; Alice grades the same shot on main, at the same time
    def member():
        fb.cmd('clip.audioGain', clips=[aid], mode='set', db=-9.0)
        fb.cmd('timeline.trim', clip=vid, edge='in', deltaFrames=15)

    def owner():
        for p_, v in (('exposure', 0.9), ('temperature', 45.0), ('saturation', 150.0), ('contrast', 25.0)):
            fa.cmd('effects.setParam', clip=v0['id'], effect='lumetri', param=p_, value=v)
    errs = both(owner, member)
    run.check('2 edits ran', not errs, errs)
    his_cut = fb.find(vid)['edit']['cut']
    run.check('2 his window has his level and trim at once', abs((gain(fb.find(aid)) or 0) + 9) < 1e-6 and his_cut != v0['edit']['cut'])
    ms = wait_for(lambda: abs((exposure(fb.find(vid)) or 0) - 0.9) < 1e-6, 60)
    run.check('2 her grade reaches his window', ms is not None, f'{ms} ms')
    run.check('2 his work waits for her', wait_for(lambda: pend() and rv(kb)['mine']['state'] == 'waiting', 60) is not None, rv(kb)['mine'])
    time.sleep(3)
    run.check('2 her window does not take his level, trim or cut', abs(gain(fa.find(aid)) or 0) < 1e-6 and not any(fa.find(i) for i in his_new) and fa.find(vid)['edit']['cut'] == v0['edit']['cut'],
              {'gain': gain(fa.find(aid)), 'his_new_clips_in_her_window': [i for i in his_new if fa.find(i)], 'her_first_shot_cut': fa.find(vid)['edit']['cut'], 'before': v0['edit']['cut']})
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shots(run, (fa, fb), '2-member-edited', 'Member1 set -9 dB and trimmed while Alice graded: he has both, she has only her grade; her review lists his change')

    # 3. approve
    ka.call(op='approve', project=pid, member=m1['account'])
    ms = wait_for(lambda: fa.find(vid) is not None and fa.find(vid)['edit']['cut'] == his_cut and abs((gain(fa.find(aid)) or 0) + 9) < 1e-6, 60)
    run.check('3 approve: her window takes his trim, level and cut', ms is not None, f'{ms} ms')
    run.check('3 one main, he says approved', wait_for(lambda: st(kb)['head'] == rv(ka)['main'] and rv(kb)['mine']['state'] == 'approved', 60) is not None)
    run.check('3 same timeline', wait_for(lambda: fa.snap() == fb.snap(), 30) is not None)
    run.check('3 grade kept', abs((exposure(fa.find(vid)) or 0) - 0.9) < 1e-6 and abs((exposure(fb.find(vid)) or 0) - 0.9) < 1e-6)
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shots(run, (fa, fb), '3-approved', 'Alice approved: her window has his trim, level and cut with her grade')

    # 4. originals, same frame, same export
    kb.call(op='fetch-original', project=pid, key='all')
    ms = wait_for(lambda: kb.call(op='media', project=pid)['from_original'] == len(paths), 300, 0.5)
    run.check('4 his originals arrive', ms is not None, f'{ms} ms')
    time.sleep(3)
    wait_for(lambda: fa.snap() == fb.snap(), 30, 0.3)
    frames, movies = [], []
    for f in (fa, fb):
        p_ = os.path.abspath(f'{run.shots}/frame-{f.who}.png')
        f.cmd('file.exportFrame', path=p_, time=int(at * TPS), target='program')
        frames.append(file_hash(p_))
        p_ = os.path.abspath(f'{run.shots}/export-{f.who}.mp4')
        f.cmd('file.exportMedia', path=p_, format='h264', hardwareEncoding='off', range='custom', startSeconds=0.0, endSeconds=2.0, wait=True)
        movies.append({'hash': file_hash(p_), 'bytes': os.path.getsize(p_)})
    run.check('4 same frame', frames[0] == frames[1], frames)
    run.check('4 byte-identical export', movies[0]['hash'] == movies[1]['hash'], movies)
    shots(run, (fa, fb), '4-originals', 'Originals on both sides: same frame, byte-identical export')
    main3 = rv(ka)['main']

    # 5. decline
    fb.cmd('clip.audioGain', clips=[aid], mode='set', db=-3.0)
    run.check('5 his new level waits', wait_for(lambda: pend(), 60) is not None)
    ka.call(op='decline', project=pid, member=m1['account'])
    run.check('5 he says declined', wait_for(lambda: rv(kb)['mine']['state'] == 'declined', 30) is not None)
    time.sleep(3)
    run.check('5 decline: her main and window keep -9 dB', rv(ka)['main'] == main3 and abs((gain(fa.find(aid)) or 0) + 9) < 1e-6, gain(fa.find(aid)))
    run.check('5 decline: he keeps -3 dB', abs((gain(fb.find(aid)) or 0) + 3) < 1e-6, gain(fb.find(aid)))
    shots(run, (fa, fb), '5-declined', 'Alice declined -3 dB: she keeps -9 dB, he keeps -3 dB')

    # 6. Alice grades again
    fa.cmd('effects.setParam', clip=vid, effect='lumetri', param='exposure', value=0.5)
    ms = wait_for(lambda: abs((exposure(fb.find(vid)) or 0) - 0.5) < 1e-6, 60)
    run.check('6 her new grade reaches him', ms is not None, f'{ms} ms')
    run.check('6 his declined level stays', abs((gain(fb.find(aid)) or 0) + 3) < 1e-6)
    time.sleep(3)
    run.check('6 nothing offered again', rv(kb)['mine']['state'] == 'declined' and not pend(), {'state': rv(kb)['mine']['state'], 'pending': len(pend())})
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shots(run, (fa, fb), '6-owner-grades-again', 'Alice graded again on main: it reached Member1; his declined level stays')

    # 7. ids
    ia, ib = fa.ids(), fb.ids()
    run.check('7 clip ids unique on both sides', len(ia) == len(set(ia)) and len(ib) == len(set(ib)) and set(ia) == set(ib), {'alice': len(ia), 'member': len(ib)})
    # 8. Member1 edits while his kopanod is stopped (Film open): the edit reaches review
    kb.stop()
    fb.cmd('clip.audioGain', clips=[aid], mode='set', db=-6.0)
    time.sleep(2)
    kb.start()
    ms = wait_for(lambda: pend() and rv(kb)['mine']['state'] == 'waiting', 90)
    run.check('8 an edit made while his service was stopped reaches review', ms is not None, f'{ms} ms')
    run.check('8 his window keeps it', abs((gain(fb.find(aid)) or 0) + 6) < 1e-6, {'gain': gain(fb.find(aid)), 'kept_b': kb.kept(pid)})
    run.check('8 her window keeps -9 dB', abs((gain(fa.find(aid)) or 0) + 9) < 1e-6)
    for f in (fa, fb):
        f.cmd('playhead.set', seconds=at)
    shots(run, (fa, fb), '8-member-offline-edit', 'Member1 set -6 dB with his service stopped; after it started the level waits for Alice')
    run.res['status'] = {'a': st(ka), 'b': st(kb)}
finally:
    code = run.finish()
sys.exit(code)
