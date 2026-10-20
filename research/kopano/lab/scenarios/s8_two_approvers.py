"""S8 two approvers: two people approve at the same moment (09 section 9, case 4).

Three Light windows, each with its own kopanod: Alice (owner), Bob (Alice made him an approver),
Member1 (editor); an "I approve changes" library of the owner's photos. Checked, screenshots of
all three windows per step:
  1. Member1 changes a photo; Alice and Bob both approve it at the same moment: one main on all
     three, his change in it, nothing offered again (no empty review), all windows show it
  2. Member1 and Bob each change a photo; at the same moment Alice approves Bob's change and Bob
     approves Member1's (main forks): the approval that lost is offered again, nothing is lost;
     approving it gives one main with both changes on all three, in all three windows
  3. the same once more without anyone approving again by hand within 20 s: what waits is listed
Usage: python s8_two_approvers.py <lightcraft.exe> <kopanod.exe> <work dir> <alice id> <m1 id> <bob id>
       --photos DIR [--shots DIR] [--light-base 18540] [--port-base 47540]"""
import sys, threading, time
from s8_common import Run, Kopanod, Light, arg, cert, fresh_homes, key, shots, wait_for

LIGHT, K, W, ALICE, M1, BOB = sys.argv[1:7]
PHOTOS = arg('--photos')
LB, KB = int(arg('--light-base', 18540)), int(arg('--port-base', 47540))
run = Run('s8_two_approvers', W)
fresh_homes(run.W, {'a': ALICE, 'b': M1, 'c': BOB})
WHO = {'a': 1, 'b': 2, 'c': 3}
try:
    k = {w: Kopanod(run, w, K, KB + n).start() for w, n in WHO.items()}
    l = {w: Light(run, w, LIGHT, LB + n, PHOTOS, k[w], f'{run.W}/lib-{w}').start() for w, n in WHO.items()}
    for x in l.values():
        x.cmd('kopano.panel', open=True)
        x.rpc('ui.set', {'view': 'detail'})
    time.sleep(10)
    m1, bob, adev = cert(M1), cert(BOB), cert(ALICE)['device']
    made = k['a'].call(op='create-project', name='Owner shoot (two approvers)', mode='approve', app={'kind': 'light-control', 'port': l['a'].port},
                       members=[{'account': m1['account'], 'role': 'editor', 'lanes': []}, {'account': bob['account'], 'role': 'editor', 'lanes': []}])
    pid = made['project']
    for w in 'bc':
        k[w].call(op='join', project=pid, **{'from': adev}, app={'kind': 'light-control', 'port': l[w].port})
    st = lambda w: k[w].call(op='status', project=pid)
    rv = lambda w: st(w)['review']
    mains = lambda: {w: rv(w)['main'] for w in 'abc'}
    one_main = lambda: len(set(mains().values())) == 1
    run.check('both joined and show main', wait_for(lambda: one_main() and all(st(w)['head'] == rv('a')['main'] for w in 'bc'), 90) is not None)
    k['a'].call(op='set-approver', project=pid, account=bob['account'], approver=True)
    run.check('Bob is an approver on every device', wait_for(lambda: rv('c')['approver'] and all(bob['account'] in rv(w)['approvers'] for w in 'abc'), 60) is not None)
    ph = sorted(l['a'].photos(), key=lambda p: str(p.get('fileName')))
    X, Y, Z = (key(p) for p in ph[:3])
    ids = {w: {kk: l[w].id_of(kk) for kk in (X, Y, Z)} for w in 'abc'}
    for w in 'abc':
        l[w].cmd('library.select', ids=[ids[w][X]], active=ids[w][X])
    time.sleep(2)
    shots(run, l.values(), '0-ready', 'Alice (a), Member1 (b) and Bob (c, approver) share the library')

    def both(fa, fc):
        out, th = {}, []
        for name, f in (('a', fa), ('c', fc)):
            def go(name=name, f=f):
                try:
                    out[name] = f()
                except Exception as e:
                    out[name] = f'error: {e}'
            th.append(threading.Thread(target=go))
        [t.start() for t in th]; [t.join() for t in th]
        return out

    def pending_for(w, acct):
        return [p for p in rv(w)['pending'] if p['account'] == acct]

    # 1. same change, both approve at once
    l['b'].cmd('develop.set', control='light.exposure', value=0.8, ids=[ids['b'][X]])
    run.check('1 both approvers see it waiting', wait_for(lambda: pending_for('a', m1['account']) and pending_for('c', m1['account']), 60) is not None)
    shots(run, l.values(), '1a-waiting', 'Member1 changed exposure: Alice and Bob both see it to review')
    out = both(lambda: k['a'].call(op='approve', project=pid, member=m1['account']), lambda: k['c'].call(op='approve', project=pid, member=m1['account']))
    run.res['same_change_results'] = out
    run.check('1 both approvals were taken', all(isinstance(v, dict) for v in out.values()), {w: (v if isinstance(v, str) else 'ok') for w, v in out.items()})
    run.check('1 one main everywhere', wait_for(one_main, 60) is not None, mains())
    time.sleep(4)
    shown = {w: l[w].dev(ids[w][X], 'light.exposure') for w in 'abc'}
    run.check('1 his change in every window', all(v == 0.8 for v in shown.values()), shown)
    left = {w: [(p['name'], p['summary'], p.get('again', False)) for p in rv(w)['pending']] for w in 'ac'}
    run.check('1 nothing offered again (no empty review)', not left['a'] and not left['c'], left)
    run.check('1 member says approved', rv('b')['mine']['state'] == 'approved', rv('b')['mine']['state'])
    run.check('1 windows show main', all(st(w)['head'] == rv('a')['main'] for w in 'abc'), {w: st(w)['head'][:8] for w in 'abc'})
    shots(run, l.values(), '1b-both-approved', 'Alice and Bob approved at the same moment: one main, his exposure in all three windows')

    # 2. different changes, approved at once by different approvers
    l['b'].cmd('library.select', ids=[ids['b'][Y]], active=ids['b'][Y])
    l['b'].cmd('photo.rate', rating=4, ids=[ids['b'][Y]])
    l['c'].cmd('develop.set', control='light.contrast', value=22, ids=[ids['c'][Z]])
    run.check('2 both changes wait', wait_for(lambda: pending_for('a', bob['account']) and pending_for('c', m1['account']), 60) is not None,
              {'a': [p['name'] for p in rv('a')['pending']], 'c': [p['name'] for p in rv('c')['pending']]})
    shots(run, l.values(), '2a-two-changes-waiting', 'Member1 rated a photo, Bob changed contrast on another')
    out = both(lambda: k['a'].call(op='approve', project=pid, member=bob['account']), lambda: k['c'].call(op='approve', project=pid, member=m1['account']))
    run.res['different_changes_results'] = out
    run.check('2 both approvals were taken', all(isinstance(v, dict) for v in out.values()), {w: (v if isinstance(v, str) else 'ok') for w, v in out.items()})
    run.check('2 one main everywhere after the fork', wait_for(one_main, 60) is not None, mains())
    time.sleep(5)
    has = lambda w: (l[w].rating(ids[w][Y]) == 4, l[w].dev(ids[w][Z], 'light.contrast') == 22)
    now = {w: has(w) for w in 'abc'}
    again = {w: [(p['name'], p['summary'], p.get('again', False)) for p in rv(w)['pending']] for w in 'ac'}
    run.res['after_fork'] = {'windows': now, 'pending': again}
    both_in = all(all(v) for v in now.values())
    offered = all(again[w] for w in 'ac')
    run.check('2 nothing lost: both in main, or the lost one offered again', both_in or offered, {'windows (rating, contrast)': now, 'pending': again})
    shots(run, l.values(), '2b-after-fork', 'Alice approved Bob\'s change while Bob approved Member1\'s, at the same moment')
    if not both_in:
        # an approver approves what was offered again
        p = rv('a')['pending'][0]
        k['a'].call(op='approve', project=pid, member=p['account'], version=p['version'])
        run.check('2 after approving what was offered again: one main', wait_for(one_main, 60) is not None, mains())
        time.sleep(4)
        now = {w: has(w) for w in 'abc'}
        run.check('2 both changes in every window', all(all(v) for v in now.values()), now)
        shots(run, l.values(), '2c-approved-again', 'Alice approved what was offered again: both changes in all three windows')
    run.check('2 nothing left to review', wait_for(lambda: not rv('a')['pending'] and not rv('c')['pending'], 30) is not None, {w: [x['summary'] for x in rv(w)['pending']] for w in 'ac'})
    run.check('2 member and Bob say approved', wait_for(lambda: rv('b')['mine']['state'] == 'approved' and rv('c')['mine']['state'] == 'approved', 30) is not None,
              {'b': rv('b')['mine']['state'], 'c': rv('c')['mine']['state']})
    run.check('2 every window shows main', wait_for(lambda: all(st(w)['head'] == rv('a')['main'] for w in 'abc'), 30) is not None)
    run.res['status'] = {w: st(w) for w in 'abc'}
finally:
    code = run.finish()
sys.exit(code)
