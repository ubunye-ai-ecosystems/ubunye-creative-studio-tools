"""S8 Light restart: the Light WINDOWS restart (not only kopanod) in an "I approve changes"
library; no edit may be lost and none may reach main without an approval (09 section 9, case 3;
the reconcile-on-connect data-loss class of S3b).

Two Light windows, each its own library on disk and its own kopanod, the owner's photos.
Checked, screenshots of both windows per step:
  1. Member1 edits and closes Light at once (before kopanod's next look); he opens it again: his
     edit is in his window and reaches Alice's review; nothing put back
  2. Alice's Light is closed; Member1 edits another photo; Alice opens Light again: her window
     shows main (his edit not taken), her review lists both photos
  3. Alice presses View (his version shows in her window) and her Light closes while she looks;
     she opens it again: main is back in her window, main did not move. The same with Light AND
     her kopanod stopped while she looks
  4. Member1's Light is closed; Alice edits main and approves his work; he opens Light again:
     his window has main (her edit and his approved work), his panel says approved
  5. Both windows restart at the same moment; then both edit: hers reaches him, his waits for her
Usage: python s8_light_restart.py <lightcraft.exe> <kopanod.exe> <work dir> <alice id> <m1 id>
       --photos DIR [--shots DIR] [--light-base 18520] [--port-base 47520]"""
import sys, threading, time
from s8_common import Run, Kopanod, Light, arg, cert, fresh_homes, key, shots, wait_for

LIGHT, K, W, ALICE, M1 = sys.argv[1:6]
PHOTOS = arg('--photos')
LB, KB = int(arg('--light-base', 18520)), int(arg('--port-base', 47520))
run = Run('s8_light_restart', W)
fresh_homes(run.W, {'a': ALICE, 'b': M1})


def open_light(who, k):
    l = Light(run, who, LIGHT, LB + (1 if who == 'a' else 2), PHOTOS, k, f'{run.W}/lib-{who}').start()
    l.cmd('kopano.panel', open=True)
    l.rpc('ui.set', {'view': 'detail'})
    return l


try:
    ka, kb = Kopanod(run, 'a', K, KB + 1).start(), Kopanod(run, 'b', K, KB + 2).start()
    la, lb = open_light('a', ka), open_light('b', kb)
    time.sleep(10)
    m1, adev = cert(M1), cert(ALICE)['device']
    made = ka.call(op='create-project', name='Owner shoot (window restarts)', mode='approve', app={'kind': 'light-control', 'port': la.port},
                   members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    kb.call(op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': lb.port})
    st = lambda k: k.call(op='status', project=pid)
    rv = lambda k: st(k)['review']
    pend = lambda: [p for p in rv(ka)['pending'] if p['account'] == m1['account']]
    run.check('member window shows main after joining', wait_for(lambda: st(kb)['head'] == rv(ka)['main'], 60) is not None)
    ph = sorted(la.photos(), key=lambda p: str(p.get('fileName')))
    X, Y, Z, V = (key(p) for p in ph[:4])
    ida = lambda k_: la.id_of(k_)
    idb = lambda k_: lb.id_of(k_)
    sel = lambda l, i: l.cmd('library.select', ids=[i], active=i)
    sel(la, ida(X)); sel(lb, idb(X))
    time.sleep(2)
    shots(run, (la, lb), '0-joined', 'Member1 joined; both windows show main')
    main0 = rv(ka)['main']

    # 1. edit, close at once, open again
    lb.cmd('develop.set', control='light.exposure', value=1.2, ids=[idb(X)])
    time.sleep(0.3)
    lb.quit()
    lb = open_light('b', kb)
    run.check('1 his edit survived his window restart', lb.dev(idb(X), 'light.exposure') == 1.2, lb.dev(idb(X), 'light.exposure'))
    ms = wait_for(lambda: pend() and pend()[0]['photos'] == 1, 60)
    run.check('1 it reaches review', ms is not None, f'{ms} ms after the window opened')
    run.check('1 still in his window after relinking', wait_for(lambda: lb.dev(idb(X), 'light.exposure') == 1.2, 5) is not None and kb.kept(pid) == 0, {'kept': kb.kept(pid)})
    run.check('1 owner main and window unchanged', rv(ka)['main'] == main0 and (la.dev(ida(X), 'light.exposure') or 0) == 0)
    sel(lb, idb(X))
    time.sleep(1)
    shots(run, (la, lb), '1-member-window-restarted', 'Member1 edited and closed Light at once, then opened it: the edit is there and waits for Alice')

    # 2. owner's window closed while he edits
    la.quit()
    sel(lb, idb(Y))
    lb.cmd('photo.rate', rating=5, ids=[idb(Y)])
    run.check('2 his second edit reaches her service while her window is closed', wait_for(lambda: pend() and pend()[0]['photos'] == 2, 60) is not None)
    la = open_light('a', ka)
    time.sleep(4)  # relinked by now
    run.check('2 her window shows main, his edits not taken', (la.rating(ida(Y)) or 0) == 0 and (la.dev(ida(X), 'light.exposure') or 0) == 0,
              {'stars': la.rating(ida(Y)), 'exposure': la.dev(ida(X), 'light.exposure')})
    run.check('2 main unchanged', rv(ka)['main'] == main0 and st(ka)['head'] == main0)
    run.check('2 her review lists both photos', pend() and pend()[0]['photos'] == 2)
    sel(la, ida(Y)); sel(lb, idb(Y))
    time.sleep(1)
    shots(run, (la, lb), '2-owner-window-restarted', 'Alice\'s Light was closed while Member1 rated a photo; reopened: her window shows main, both photos wait for review')

    # 3a. her window closes while she views his version
    sel(la, ida(X))
    la.cmd('kopano.view', project=pid, member=m1['account'], show='member')
    run.check('3 view shows his version', wait_for(lambda: la.dev(ida(X), 'light.exposure') == 1.2, 20) is not None)
    shots(run, (la, lb), '3a-viewing', 'Alice pressed View: his version in her window')
    la.quit()
    la = open_light('a', ka)
    time.sleep(1)
    shots(run, (la, lb), '3b-reopened-while-viewing', 'Her Light closed while she was looking; reopened')
    la.cmd('kopano.view', project=pid, show='off')  # what her panel's Done does; the panel offers it again
    ok = wait_for(lambda: (la.dev(ida(X), 'light.exposure') or 0) == 0 and (la.rating(ida(Y)) or 0) == 0, 20)
    time.sleep(3)
    run.check('3 window restart while viewing: main back in her window', ok is not None and (la.dev(ida(X), 'light.exposure') or 0) == 0,
              {'exposure': la.dev(ida(X), 'light.exposure'), 'stars': la.rating(ida(Y))})
    run.check('3 window restart while viewing: main did not move', rv(ka)['main'] == main0 and st(ka)['head'] == main0)
    # 3b. her Light AND her kopanod go down while she looks
    la.cmd('kopano.view', project=pid, member=m1['account'], show='member')
    wait_for(lambda: la.dev(ida(X), 'light.exposure') == 1.2, 20)
    la.quit()
    ka.stop()
    la = open_light('a', ka)
    run.check('3 her library kept his version while nothing ran (what relinking must undo)', la.dev(ida(X), 'light.exposure') == 1.2, la.dev(ida(X), 'light.exposure'))
    ka.start()
    ok = wait_for(lambda: (la.dev(ida(X), 'light.exposure') or 0) == 0 and (la.rating(ida(Y)) or 0) == 0, 60)
    time.sleep(4)
    run.check('3 window and service restart while viewing: main back in her window', ok is not None and (la.dev(ida(X), 'light.exposure') or 0) == 0,
              {'exposure': la.dev(ida(X), 'light.exposure'), 'stars': la.rating(ida(Y))})
    run.check('3 window and service restart while viewing: nothing approved', rv(ka)['main'] == main0 and st(ka)['head'] == main0 and pend() and pend()[0]['photos'] == 2,
              {'main_moved': rv(ka)['main'] != main0, 'pending': [p['summary'] for p in pend()]})
    run.check('3 member still waiting', wait_for(lambda: rv(kb)['mine']['state'] == 'waiting', 30) is not None, rv(kb)['mine']['state'])
    sel(la, ida(X))
    time.sleep(1)
    shots(run, (la, lb), '3c-after-service-restart-while-viewing', 'Her Light and service went down while she looked; after both started: main in her window, nothing approved')

    # 4. his window closed while she edits main and approves
    lb.quit()
    la.cmd('develop.set', control='light.contrast', value=-25, ids=[ida(Z)])
    time.sleep(1.5)
    la.cmd('kopano.approve', project=pid, member=m1['account'])
    run.check('4 approve: her window follows', wait_for(lambda: la.dev(ida(X), 'light.exposure') == 1.2 and la.rating(ida(Y)) == 5, 30) is not None)
    main4 = rv(ka)['main']
    time.sleep(3)
    lb = open_light('b', kb)
    ms = wait_for(lambda: lb.dev(idb(Z), 'light.contrast') == -25 and st(kb)['head'] == main4, 60)
    run.check('4 his reopened window has main (her edit)', ms is not None, {'ms': ms, 'contrast': lb.dev(idb(Z), 'light.contrast'), 'head_is_main': st(kb)['head'] == main4})
    run.check('4 and his approved work', lb.dev(idb(X), 'light.exposure') == 1.2 and lb.rating(idb(Y)) == 5)
    run.check('4 his panel says approved', wait_for(lambda: rv(kb)['mine']['state'] == 'approved', 20) is not None, rv(kb)['mine']['state'])
    sel(la, ida(Z)); sel(lb, idb(Z))
    time.sleep(1)
    shots(run, (la, lb), '4-member-reopened-after-approval', 'Member1\'s Light was closed while Alice edited and approved; reopened: his window is main')

    # 5. both windows restart at once, then both edit
    kept5 = (ka.kept(pid), kb.kept(pid))
    th = [threading.Thread(target=la.quit), threading.Thread(target=lb.quit)]
    [t.start() for t in th]; [t.join() for t in th]
    out = {}
    th = [threading.Thread(target=lambda: out.__setitem__('a', open_light('a', ka))), threading.Thread(target=lambda: out.__setitem__('b', open_light('b', kb)))]
    [t.start() for t in th]; [t.join() for t in th]
    la, lb = out['a'], out['b']
    time.sleep(4)
    main5 = rv(ka)['main']
    run.check('5 after both restarted: main unchanged, nothing waiting', main5 == main4 and not pend(), {'pending': [p['summary'] for p in pend()]})
    run.check('5 windows still hold the work', la.dev(ida(X), 'light.exposure') == 1.2 and lb.dev(idb(Z), 'light.contrast') == -25)
    la.cmd('develop.set', control='light.exposure', value=0.4, ids=[ida(V)])
    lb.cmd('develop.set', control='light.contrast', value=40, ids=[idb(Z)])
    run.check('5 her edit reaches him', wait_for(lambda: lb.dev(idb(V), 'light.exposure') == 0.4, 60) is not None)
    run.check('5 his edit waits for her', wait_for(lambda: pend() and pend()[0]['photos'] == 1, 60) is not None)
    run.check('5 her window does not take it', la.dev(ida(Z), 'light.contrast') == -25)
    run.check('5 nothing put aside on either side', (ka.kept(pid), kb.kept(pid)) == kept5, {'before': kept5, 'after': (ka.kept(pid), kb.kept(pid))})
    sel(la, ida(Z)); sel(lb, idb(Z))
    time.sleep(1)
    shots(run, (la, lb), '5-both-restarted', 'Both windows restarted at once; then her edit reached him and his waits for her')
    run.res['status'] = {'a': st(ka), 'b': st(kb)}
finally:
    code = run.finish()
sys.exit(code)
