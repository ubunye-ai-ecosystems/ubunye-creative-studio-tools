"""S8 Photo approve: Ubunye Photo in an "I approve changes" project (09 section 9, case 1).

Two Photo windows, each with its own kopanod (own KOPANO_HOME and KOPANO_API_PORT, own device);
Alice opens the owner's DSC08379 as a .pcraft document, Member1's window starts blank and joins.
Checked, screenshots of both windows per step:
  1. Member1 adds a layer and paints two strokes: his window has them, Alice's main and window do
     not change, her review lists his change, his says waiting
  2. Alice presses View: his strokes show in her window (tiles fetched); Before; Done: unchanged
  3. Alice paints meanwhile on main: Member1's window gets it and keeps his strokes
  4. Alice approves: her window becomes his (tiles fetched), main moved, he says approved
  5. Member1 paints again, Alice declines: her main and window stay, he keeps the stroke
  6. Alice paints again: it reaches him; his declined stroke stays; nothing offered again
  7. both kopanod stop and start: heads, review state and windows kept; his next stroke waits again
  8. Member1 paints while his kopanod is stopped: the stroke stays in his window and reaches review
Usage: python s8_photo_approve.py <photocraft.exe> <kopanod.exe> <work dir> <alice id> <m1 id>
       --doc <owner .pcraft> [--shots DIR] [--photo-base 18580] [--port-base 47580]"""
import json, os, sys, time
from s8_common import Run, Kopanod, Photo, arg, cert, fresh_homes, shape, layer_id, moved_kb, shots, wait_for

PHOTO, K, W, ALICE, M1 = sys.argv[1:6]
DOC = arg('--doc')
PB, KB = int(arg('--photo-base', 18580)), int(arg('--port-base', 47580))
run = Run('s8_photo_approve', W)
fresh_homes(run.W, {'a': ALICE, 'b': M1})
try:
    ka, kb = Kopanod(run, 'a', K, KB + 1).start(), Kopanod(run, 'b', K, KB + 2).start()
    wa = Photo(run, 'a', PHOTO, PB + 1, DOC).start(True)
    wb = Photo(run, 'b', PHOTO, PB + 2, DOC).start(False)
    time.sleep(10)
    m1, adev = cert(M1), cert(ALICE)['device']
    made = ka.call(op='create-project', name='Owner photo (approve)', mode='approve', app=wa.binding(),
                   members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    run.check('created in approve mode', made.get('mode') == 'approve', {'items': made['items']})
    kb0 = moved_kb(kb)
    kb.call(op='join', project=pid, **{'from': adev}, app=wb.binding())
    st = lambda k: k.call(op='status', project=pid)
    rv = lambda k: st(k)['review']
    pend = lambda: [p for p in rv(ka)['pending'] if p['account'] == m1['account']]
    same = lambda: shape(wa.export()) == shape(wb.export())
    ms = wait_for(same, 120)
    run.check('member window became the owner document', ms is not None, {'ms': ms, 'tiles': len(shape(wb.export())[1]), 'mb': round((moved_kb(kb) - kb0) / 1000, 2)})
    run.check('member head is main', wait_for(lambda: st(kb)['head'] == rv(ka)['main'], 30) is not None)
    time.sleep(1.5)
    shots(run, (wa, wb), '0-joined', 'Member1 joined Alice\'s "I approve changes" project: his window is her document')
    main0, a0 = rv(ka)['main'], shape(wa.export())

    # 1. Member1 paints
    wb.cmd('layer.new.layer', name='Retouch M1')
    wb.cmd('paint.stroke', points=[[1500, 600, 1], [2100, 700, 1], [2600, 650, 1]], size=80, color='#ff3f9a')
    wb.cmd('paint.stroke', points=[[400, 1500, 1], [900, 1580, 1]], size=60, color='#3fff7a')
    b1 = shape(wb.export())
    run.check('1 his window has his layer and strokes', 'Retouch M1' in [l[0] for l in b1[0]] and len(set(b1[1]) - set(a0[1])) > 0, {'new_tiles': len(set(b1[1]) - set(a0[1]))})
    ms = wait_for(lambda: pend(), 60)
    run.time('member_stroke_to_pending_ms', ms)
    run.check('1 her review lists his change', ms is not None, pend()[0]['summary'] if pend() else rv(ka)['pending'])
    run.check('1 he says waiting', wait_for(lambda: rv(kb)['mine']['state'] == 'waiting', 20) is not None, rv(kb)['mine'])
    time.sleep(2)
    run.check('1 her main and window unchanged', rv(ka)['main'] == main0 and st(ka)['head'] == main0 and shape(wa.export()) == a0)
    shots(run, (wa, wb), '1-member-painted', 'Member1 added a layer and painted: his window has it, Alice\'s does not; her review lists it')

    # 2. View, Before, Done
    a_kb = moved_kb(ka)
    ka.call(op='view', project=pid, member=m1['account'], show='member')
    ms = wait_for(lambda: shape(wa.export()) == b1, 60)
    run.check('2 view: his strokes show in her window (tiles fetched)', ms is not None, {'ms': ms, 'mb_fetched': round((moved_kb(ka) - a_kb) / 1000, 2)})
    shots(run, (wa, wb), '2a-view-after', 'Alice pressed View: Member1\'s strokes in her window')
    ka.call(op='view', project=pid, member=m1['account'], show='main')
    run.check('2 before: her version again', wait_for(lambda: shape(wa.export()) == a0, 30) is not None)
    shots(run, (wa, wb), '2b-view-before', 'Before: her version')
    ka.call(op='view', project=pid, show='off')
    time.sleep(3)
    run.check('2 looking saved nothing', rv(ka)['main'] == main0 and shape(wa.export()) == a0 and rv(ka)['viewing'] is None)

    # 3. Alice paints meanwhile on main
    wa.cmd('layer.newAdjustmentLayer.exposure', exposure=0.4, offset=0.0, gamma=1.0)
    run.check('3 her edit moves main', wait_for(lambda: rv(ka)['main'] != main0, 30) is not None)
    ms = wait_for(lambda: any(json.loads(l[1] or 'null') and 'exposure' in str(l[1]) for l in shape(wb.export())[0]) and 'Retouch M1' in [l[0] for l in shape(wb.export())[0]], 60)
    run.check('3 her edit reaches his window, his strokes stay', ms is not None, f'{ms} ms')
    run.check('3 his work still waits', pend() and rv(kb)['mine']['state'] == 'waiting')
    time.sleep(1.5)
    shots(run, (wa, wb), '3-owner-edit-reaches-member', 'Alice added an Exposure layer on main: it reached Member1, his strokes stay')

    # 4. approve
    a_kb = moved_kb(ka)
    t0 = time.time()
    ka.call(op='approve', project=pid, member=m1['account'])
    ms = wait_for(same, 60)
    run.time('approve_to_owner_window_ms', ms)
    run.check('4 approve: her window follows (his strokes, her exposure)', ms is not None, {'ms': ms, 'mb_fetched': round((moved_kb(ka) - a_kb) / 1000, 2)})
    run.check('4 he says approved', wait_for(lambda: rv(kb)['mine']['state'] == 'approved', 30) is not None)
    run.check('4 his window is main', wait_for(lambda: st(kb)['head'] == rv(ka)['main'], 30) is not None)
    time.sleep(1.5)
    shots(run, (wa, wb), '4-approved', 'Alice approved: her window has Member1\'s strokes')
    main4, a4 = rv(ka)['main'], shape(wa.export())

    # 5. paint, decline
    wb.cmd('layer.select', layer=layer_id(wb, 'Retouch M1'))
    wb.cmd('paint.stroke', points=[[2000, 200, 1], [2900, 260, 1]], size=60, color='#c03020')
    b5 = shape(wb.export())
    run.check('5 his next stroke waits', wait_for(lambda: pend(), 60) is not None)
    ka.call(op='decline', project=pid, member=m1['account'])
    run.check('5 he says declined', wait_for(lambda: rv(kb)['mine']['state'] == 'declined', 30) is not None)
    time.sleep(3)
    run.check('5 decline: her main and window unchanged', rv(ka)['main'] == main4 and shape(wa.export()) == a4)
    run.check('5 decline: he keeps the stroke', shape(wb.export()) == b5)
    shots(run, (wa, wb), '5-declined', 'Alice declined his red stroke: her document stays, his window keeps it')

    # 6. Alice paints again
    wa.cmd('layer.new.layer', name='Retouch Alice')
    wa.cmd('paint.stroke', points=[[150, 300, 1], [500, 380, 1]], size=70, color='#3fa7ff')
    ms = wait_for(lambda: 'Retouch Alice' in [l[0] for l in shape(wb.export())[0]] and set(shape(wa.export())[1]) <= set(shape(wb.export())[1]), 60)
    run.check('6 her stroke reaches him', ms is not None, f'{ms} ms')
    run.check('6 his declined stroke stays', set(b5[1]) - set(a4[1]) <= set(shape(wb.export())[1]))
    time.sleep(2)
    run.check('6 declined work not offered again', rv(kb)['mine']['state'] == 'declined' and not pend(), {'state': rv(kb)['mine']['state'], 'pending': len(pend())})
    shots(run, (wa, wb), '6-owner-paints-again', 'Alice painted on main: it reached Member1; his declined stroke stays in his window')

    # 7. restart both kopanod
    before = {'ha': st(ka)['head'], 'hb': st(kb)['head'], 'ra': rv(ka), 'rb': rv(kb), 'sa': shape(wa.export()), 'sb': shape(wb.export())}
    ka.stop(); kb.stop()
    time.sleep(1)
    ka.start(); kb.start()
    ok = wait_for(lambda: st(ka)['head'] == before['ha'] and st(kb)['head'] == before['hb'], 60)
    run.check('7 restart: heads kept', ok is not None)
    run.check('7 restart: review kept', rv(ka)['main'] == before['ra']['main'] and rv(kb)['mine']['state'] == 'declined')
    time.sleep(8)
    run.check('7 restart: windows unchanged', shape(wa.export()) == before['sa'] and shape(wb.export()) == before['sb'],
              {'a_same': shape(wa.export()) == before['sa'], 'b_same': shape(wb.export()) == before['sb'], 'kept_a': ka.kept(pid), 'kept_b': kb.kept(pid)})
    wb.cmd('layer.select', layer=layer_id(wb, 'Retouch M1'))
    wb.cmd('paint.stroke', points=[[2400, 1500, 1], [2800, 1580, 1]], size=70, color='#ffd23f')
    run.check('7 restart: his next stroke waits for review', wait_for(lambda: pend() and rv(kb)['mine']['state'] == 'waiting', 60) is not None)
    run.check('7 restart: her window unchanged', shape(wa.export()) == before['sa'])
    time.sleep(1.5)
    shots(run, (wa, wb), '7-after-restart', 'Both services restarted: state kept; his next stroke waits for review')
    # 8. Member1 paints while his kopanod is stopped (Photo open): the strokes reach review
    ka.call(op='decline', project=pid, member=m1['account'])
    wait_for(lambda: rv(kb)['mine']['state'] == 'declined', 30)
    kb.stop()
    wb.cmd('layer.select', layer=layer_id(wb, 'Retouch M1'))
    wb.cmd('paint.stroke', points=[[300, 900, 1], [2700, 940, 1]], size=40, color='#ffffff')
    b8 = shape(wb.export())
    time.sleep(2)
    shots(run, (wa, wb), '8a-member-offline-stroke', 'Member1 painted a white stroke with his service stopped')
    kb.start()
    ms = wait_for(lambda: pend() and rv(kb)['mine']['state'] == 'waiting', 90)
    run.check('8 offline stroke reaches review after his service starts', ms is not None, f'{ms} ms')
    run.check('8 his window keeps the offline stroke', shape(wb.export()) == b8, {'kept_b': kb.kept(pid)})
    run.check('8 her window unchanged', shape(wa.export()) == before['sa'])
    ka.call(op='approve', project=pid, member=m1['account'])
    run.check('8 approved: both windows match', wait_for(same, 60) is not None)
    time.sleep(1.5)
    shots(run, (wa, wb), '8b-offline-stroke-approved', 'His service started: the stroke waited for Alice; she approved it')
    run.res['media_refusals'] = {w: k.log_count('media: refused') for w, k in (('a', ka), ('b', kb))}
    run.check('no media refused', sum(run.res['media_refusals'].values()) == 0, run.res['media_refusals'])
    run.res['status'] = {'a': st(ka), 'b': st(kb)}
finally:
    for w in ('wa', 'wb'):
        try:
            globals()[w].quit()
        except Exception:
            pass
    code = run.finish()
sys.exit(code)
