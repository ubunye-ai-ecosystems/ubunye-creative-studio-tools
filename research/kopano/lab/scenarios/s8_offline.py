"""S8 offline: edits made while kopanod is not running are not lost (09 section 9, case 6).

Two Light windows (each its own library on disk, as people run it), each with its own kopanod,
the owner's photos, an "I approve changes" library. Checked, screenshots of both windows per step:
  1. Member1's kopanod stops; Light stays open; he edits two photos; Alice edits a third on main.
     His kopanod starts again: his window keeps his edits (not put back), they reach Alice's
     "Changes to review", Alice's main and window do not take them, and he gets Alice's edit.
  2. Alice approves: her window follows.
  3. Member1's kopanod stops, he edits, closes Light and opens it again (his library keeps the
     edit), then kopanod starts: the edit still reaches review, nothing is put back.
  4. The owner offline: Alice's kopanod stops, she edits, it starts: her edit is on main and
     reaches Member1's window.
Usage: python s8_offline.py <lightcraft.exe> <kopanod.exe> <work dir> <alice id> <m1 id> --photos DIR
       [--shots DIR] [--light-base 18500] [--port-base 47500]"""
import os, sys, time
from s8_common import Run, Kopanod, Light, arg, cert, fresh_homes, key, shots, wait_for

LIGHT, K, W, ALICE, M1 = sys.argv[1:6]
PHOTOS = arg('--photos')
LB, KB = int(arg('--light-base', 18500)), int(arg('--port-base', 47500))
run = Run('s8_offline', W)
fresh_homes(run.W, {'a': ALICE, 'b': M1})
try:
    ka, kb = Kopanod(run, 'a', K, KB + 1).start(), Kopanod(run, 'b', K, KB + 2).start()
    la = Light(run, 'a', LIGHT, LB + 1, PHOTOS, ka, f'{run.W}/lib-a').start()
    lb = Light(run, 'b', LIGHT, LB + 2, PHOTOS, kb, f'{run.W}/lib-b').start()
    time.sleep(10)  # both services online
    m1, adev = cert(M1), cert(ALICE)['device']
    made = ka.call(op='create-project', name='Owner shoot (offline test)', mode='approve', app={'kind': 'light-control', 'port': la.port},
                   members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    kb.call(op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': lb.port})
    rv = lambda k: k.call(op='status', project=pid)['review']
    st = lambda k: k.call(op='status', project=pid)
    run.check('member window shows main after joining', wait_for(lambda: st(kb)['head'] == rv(ka)['main'], 60) is not None)
    for l in (la, lb):
        l.cmd('kopano.panel', open=True)
        l.rpc('ui.set', {'view': 'detail'})
    pa = la.photos()
    ph = sorted(pa, key=lambda p: str(p.get('fileName')))
    X, Y, Z, V = (key(p) for p in ph[:4])
    A = {k_: la.id_of(k_) for k_ in (X, Y, Z, V)}
    B = {k_: lb.id_of(k_) for k_ in (X, Y, Z, V)}
    la.cmd('library.select', ids=[A[X]], active=A[X])
    lb.cmd('library.select', ids=[B[X]], active=B[X])
    time.sleep(2)
    shots(run, (la, lb), '0-joined', 'Member1 joined; both windows show main')
    main0 = rv(ka)['main']

    # 1. Member1 edits with his kopanod stopped; Alice edits main meanwhile
    kb.stop()
    lb.cmd('develop.set', control='light.exposure', value=0.9, ids=[B[X]])
    lb.cmd('library.select', ids=[B[Y]], active=B[Y])
    lb.cmd('photo.rate', rating=3, ids=[B[Y]])
    lb.cmd('library.select', ids=[B[X]], active=B[X])
    la.cmd('develop.set', control='light.contrast', value=-20, ids=[A[Z]])
    run.check('owner edit lands on main while the member is away', wait_for(lambda: rv(ka)['main'] != main0, 20) is not None)
    time.sleep(3)
    shots(run, (la, lb), '1a-member-offline-edits', 'Member1 edited 2 photos with his service stopped; Alice edited a third on main')
    t0 = time.time()
    kb.start()
    ms = wait_for(lambda: any(p['account'] == m1['account'] for p in rv(ka)['pending']), 90)
    run.time('offline_edit_to_pending_ms', round((time.time() - t0) * 1000) if ms is not None else None)
    pend = [p for p in rv(ka)['pending'] if p['account'] == m1['account']]
    run.check('offline edits reach review after his service starts', ms is not None and pend and pend[0]['photos'] == 2, pend[0]['summary'] if pend else rv(ka)['pending'])
    run.check('member window keeps his offline edits', lb.dev(B[X], 'light.exposure') == 0.9 and lb.rating(B[Y]) == 3,
              {'exposure': lb.dev(B[X], 'light.exposure'), 'stars': lb.rating(B[Y])})
    run.check('member gets the owner edit made meanwhile', wait_for(lambda: lb.dev(B[Z], 'light.contrast') == -20, 30) is not None)
    run.check('member says waiting', rv(kb)['mine']['state'] == 'waiting', rv(kb)['mine'])
    run.check('owner window did not take them', (la.dev(A[X], 'light.exposure') or 0) == 0 and (la.rating(A[Y]) or 0) == 0)
    run.check('nothing kept aside on the member side', kb.kept(pid) == 0, kb.kept(pid))
    time.sleep(1)
    shots(run, (la, lb), '1b-member-back-online', 'His service started: his edits stay in his window and wait for Alice; he has her edit')

    # 2. approve
    la.cmd('kopano.approve', project=pid, member=m1['account'])
    ms = wait_for(lambda: la.dev(A[X], 'light.exposure') == 0.9 and la.rating(A[Y]) == 3, 30)
    run.check('approved: owner window follows', ms is not None, f'{ms} ms')
    run.check('approved: member says approved', wait_for(lambda: rv(kb)['mine']['state'] == 'approved', 30) is not None)
    shots(run, (la, lb), '2-approved', 'Alice approved the offline edits')

    # 3. offline edit, then Light closed and opened again before the service starts
    main1 = rv(ka)['main']
    kb.stop()
    lb.cmd('develop.set', control='light.exposure', value=-0.4, ids=[B[V]])
    time.sleep(1.5)
    lb.quit()
    lb = Light(run, 'b', LIGHT, LB + 2, PHOTOS, kb, f'{run.W}/lib-b').start()
    lb.cmd('kopano.panel', open=True)
    lb.rpc('ui.set', {'view': 'detail'})
    B = {k_: lb.id_of(k_) for k_ in (X, Y, Z, V)}
    run.check('his library kept the edit across the restart', lb.dev(B[V], 'light.exposure') == -0.4, lb.dev(B[V], 'light.exposure'))
    kb.start()
    ms = wait_for(lambda: any(p['account'] == m1['account'] for p in rv(ka)['pending']), 90)
    run.check('edit made offline before a Light restart reaches review', ms is not None)
    run.check('his window keeps it', lb.dev(B[V], 'light.exposure') == -0.4, lb.dev(B[V], 'light.exposure'))
    run.check('his window kept the approved work', lb.dev(B[X], 'light.exposure') == 0.9 and lb.rating(B[Y]) == 3)
    run.check('owner main unchanged by it', rv(ka)['main'] == main1 and (la.dev(A[V], 'light.exposure') or 0) == 0)
    lb.cmd('library.select', ids=[B[V]], active=B[V])
    la.cmd('library.select', ids=[A[V]], active=A[V])
    time.sleep(1.5)
    shots(run, (la, lb), '3-offline-then-light-restart', 'Member1 edited offline, closed and reopened Light, then his service started: the edit waits for review')
    la.cmd('kopano.decline', project=pid, member=m1['account'])
    run.check('declined', wait_for(lambda: rv(kb)['mine']['state'] == 'declined', 30) is not None)

    # 4. the owner offline
    ka.stop()
    la.cmd('develop.set', control='light.contrast', value=15, ids=[A[X]])
    time.sleep(2)
    ka.start()
    ms = wait_for(lambda: rv(ka)['main'] != main1 and lb.dev(B[X], 'light.contrast') == 15, 90)
    run.check('owner offline edit reaches main and the member', ms is not None, f'{ms} ms')
    run.check('owner window keeps her offline edit', la.dev(A[X], 'light.contrast') == 15)
    run.check('nothing kept aside on the owner side', ka.kept(pid) == 0, ka.kept(pid))
    run.check('member still declined, keeps his work', rv(kb)['mine']['state'] == 'declined' and lb.dev(B[V], 'light.exposure') == -0.4)
    la.cmd('library.select', ids=[A[X]], active=A[X])
    lb.cmd('library.select', ids=[B[X]], active=B[X])
    time.sleep(1.5)
    shots(run, (la, lb), '4-owner-offline-edit', 'Alice edited with her service stopped; it reached main and Member1')
    run.res['status'] = {'a': st(ka), 'b': st(kb)}
finally:
    code = run.finish()
sys.exit(code)
