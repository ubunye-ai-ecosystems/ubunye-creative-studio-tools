"""S9: signed in with ThabangVision, invited by email, joined with no ids (09 sections 3 and 4).

Two kopanod on this PC (own homes, own API ports, own device keys) against a DEV ThabangVision
(never production). Two throwaway accounts are made on the dev database first and deleted at the
end. "Link this computer" is approved the way the /link page does it (dev database, service role).
Checked, as the apps' panels would see it through the local API:
  1. a fresh kopanod says "not signed in"; sign-in gives a code and link; once approved it goes
     online by itself with the account's display name, never an email
  2. Alice shares a project and invites Bob's EMAIL: "Invite sent."; an address with no account
     gets the very same answer (no account enumeration)
  3. Bob sees it under Shared with me and joins with no ids typed; Alice's computer signs him in
  4. Alice's edit reaches Bob's window live; Bob's edit reaches Alice for approval (the project's
     default "I approve changes") and, approved, is in both
  5. People lists both by display name; no email anywhere in any answer
  6. Bob leaves: the project is gone from his computer and from Shared with me, and Alice's
     computer takes him off the members log
  7. Alice invites him again, he joins again, Alice removes him: his computer is refused
  8. Silent renewal: a new certificate in place, no restart, still online
  9. Bob's computer is unlinked on the website: at its next renewal it signs itself out and says
     why; sign out and sign in again on Alice's computer keeps her device and projects
Usage: python s9_signin.py <kopanod.exe> <work dir> --tv http://localhost:3100
       --env <ThabangVision .env.local with the DEV Supabase URL and service key> [--port-base N]"""
import json, os, secrets, shutil, socket, subprocess, sys, time, urllib.request, urllib.error

K, W = sys.argv[1], os.path.abspath(sys.argv[2])
arg = lambda n, d=None: sys.argv[sys.argv.index(n) + 1] if n in sys.argv else d
TV = arg('--tv', 'http://localhost:3100').rstrip('/')
BASE = int(arg('--port-base', 47500))
if 'thabangvision.com' in TV:
    sys.exit('S9 runs against a dev server only')
ENV = dict(l.split('=', 1) for l in open(arg('--env'), encoding='utf-8').read().splitlines() if '=' in l and not l.startswith('#'))
SB, SK = ENV['NEXT_PUBLIC_SUPABASE_URL'].strip().strip('"'), ENV['SUPABASE_SERVICE_ROLE_KEY'].strip().strip('"')
if 'bprargrqppufeuewryze' not in SB:
    sys.exit('S9 uses the DEV Supabase project only')
checks, timings, procs, users = {}, {}, {}, {}
PORT = {'a': BASE + 1, 'b': BASE + 2}
HOME = {w: os.path.join(W, w) for w in PORT}


def check(name, ok, detail=None):
    checks[name] = {'ok': bool(ok), 'detail': detail}
    print(('PASS ' if ok else 'FAIL ') + name + ('' if detail is None else f'  {detail}'), flush=True)


def sb(method, path, body=None, prefer=None):
    req = urllib.request.Request(SB + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={'apikey': SK, 'Authorization': 'Bearer ' + SK, 'Content-Type': 'application/json', **({'Prefer': prefer} if prefer else {})})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            t = r.read()
            return json.loads(t) if t else None
    except urllib.error.HTTPError as e:
        raise RuntimeError(f'{method} {path}: {e.code} {e.read()[:300]}')


def start(w, extra=None):
    log = open(os.path.join(HOME[w], 'kopanod.log'), 'a')
    env = {**os.environ, 'KOPANO_TV_URL': TV, 'KOPANO_NO_BROWSER': '1', **(extra or {})}
    procs[w] = subprocess.Popen([K, 'run', '--home', HOME[w], '--api-port', str(PORT[w])], stdout=log, stderr=log, env=env)


def stop(w):
    procs[w].terminate()
    procs[w].wait(15)


def call(w, timeout=120, **req):
    f = os.path.join(HOME[w], 'api.token')
    for _ in range(300):
        if os.path.exists(f):
            break
        time.sleep(0.1)
    for _ in range(150):
        try:
            s = socket.create_connection(('127.0.0.1', PORT[w]), timeout=timeout)
            s.sendall((json.dumps({**req, 'token': open(f).read().strip()}) + '\n').encode())
            line = s.makefile().readline()
            s.close()
            if line:
                break
        except OSError:
            pass
        time.sleep(0.2)  # the service is starting (or going online after signing in)
    r = json.loads(line)
    if not r['ok']:
        raise RuntimeError(r['error'])
    return r['result']


def attempt(w, **req):
    try:
        return True, call(w, **req)
    except RuntimeError as e:
        return False, str(e)


def wait(fn, timeout, step=0.2):
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            v = fn()
            if v:
                return round((time.time() - t0) * 1000), v
        except Exception:
            pass
        time.sleep(step)
    return None, None


def no_email(*answers):
    text = json.dumps(answers)
    return all(u['email'] not in text for u in users.values()) and '@example.invalid' not in text


def approve_link(code, user):
    rows = sb('PATCH', f'/rest/v1/kopano_link_requests?user_code=eq.{code}&status=eq.pending',
              {'status': 'approved', 'user_id': users[user]['id'], 'decided_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}, 'return=representation')
    return len(rows or [])


def sign_in(w, user):
    code = call(w, op='sign-in')
    ok_code = len(code['code']) == 9 and code['link'].startswith(TV + '/link?code=')
    hello = call(w, op='hello')
    approved = approve_link(code['code'], user)
    t0 = time.time()
    ms, h = wait(lambda: (lambda h: h if h['signed_in'] else None)(call(w, op='hello', timeout=5)), 90, 0.5)
    return ok_code and hello['signing_in']['code'] == code['code'] and approved == 1, ms, h


def toy_items(n=3):
    return {f'photo-{i:04}': {'kind': 'lab.photo', 'lane': 'develop', 'schema': 1, 'assets': [],
                              'state': {'develop': {'exposure': 0.0, 'contrast': 0.0}, 'rating': 0, 'flag': 'none'}} for i in range(n)}


shutil.rmtree(W, ignore_errors=True)
for h in HOME.values():
    os.makedirs(h)
try:
    # 0. two throwaway dev accounts (A and B), confirmed
    tag = secrets.token_hex(3)
    for u, name in (('a', 'Alice Test'), ('b', 'Bob Test')):
        email = f'kopano-s9-{u}-{tag}@example.invalid'
        r = sb('POST', '/auth/v1/admin/users', {'email': email, 'password': secrets.token_urlsafe(18), 'email_confirm': True,
                                               'user_metadata': {'full_name': name, 'display_name': name}})
        users[u] = {'id': r['id'], 'email': email, 'name': name}
        try:
            sb('PATCH', f'/rest/v1/profiles?id=eq.{r["id"]}', {'display_name': name})
        except RuntimeError:
            try:
                sb('PATCH', f'/rest/v1/profiles?id=eq.{r["id"]}', {'full_name': name})
            except RuntimeError as e:
                print('profile name not set:', e)

    # 1. fresh services: not signed in; sign in through the device flow
    start('a'); start('b')
    h = call('a', op='hello')
    check('fresh_kopanod_says_not_signed_in', h['signed_in'] is False and h['signing_in'] is None, h)
    ok, msg = attempt('a', op='shared-projects')
    check('not_signed_in_refuses_projects_in_plain_english', not ok and 'not signed in' in msg and 'kopano' not in msg.lower(), msg)
    for w in ('a', 'b'):
        ok, ms, h = sign_in(w, w)
        timings[f'{w}_signed_in_ms'] = ms
        check(f'{w}_signs_in_and_goes_online', ok and h and h['signed_in'] and h['account'] == users[w]['id'] and no_email(h), {'ms': ms, 'name': h and h['name']})
    dev_a = call('a', op='hello')['device']

    # 2. Alice shares a project and invites by email
    p = call('a', op='create-project', name=f'S9 shoot {tag}')
    pid = p['project']
    check('new_project_is_approve_mode', p['mode'] == 'approve', p['mode'])
    json.dump(toy_items(), open(os.path.join(HOME['a'], 'projects', pid[:8], 'app.json'), 'w'))
    call('a', op='set', project=pid, key='photo-0000', field='rating', value=1)
    sh = call('a', op='share', project=pid)
    check('share_registers_on_thabangvision', bool(sh.get('shared')), sh)
    inv = call('a', op='invite', project=pid, email=users['b']['email'])
    nobody = call('a', op='invite', project=pid, email=f'nobody-{tag}@example.invalid')
    check('invite_by_email_says_invite_sent', inv == {'message': 'Invite sent.'}, inv)
    check('no_account_enumeration', inv == nobody, nobody)
    ok, msg = attempt('a', op='invite', project=pid, email='not an email')
    check('a_bad_email_is_refused_in_plain_english', not ok and 'email' in msg, msg)

    # 3. Bob: Shared with me, Join with no ids
    lst = call('b', op='shared-projects')
    row = next((x for x in lst['shared'] if x['project'] == pid), None)
    check('bob_sees_it_under_shared_with_me', row is not None and row['here'] is False and row['status'] == 'invited', row)
    t0 = time.time()
    j = call('b', op='join-shared', shared=row['shared'])
    timings['join_ms'] = round((time.time() - t0) * 1000)
    check('bob_joins_with_no_ids', j['project'] == pid, {**j, 'ms': timings['join_ms']})
    ms, _ = wait(lambda: call('b', op='get', project=pid, key='photo-0000', field='rating') == 1, 30)
    check('bob_has_the_project_state', ms is not None, ms)
    lst = call('b', op='shared-projects')
    row = next(x for x in lst['shared'] if x['project'] == pid)
    check('shared_with_me_marks_it_here_and_active', row['here'] and row['status'] == 'active', row)

    # 4. live edits
    call('a', op='set', project=pid, key='photo-0001', field='develop.exposure', value=1.25)
    ms, _ = wait(lambda: call('b', op='get', project=pid, key='photo-0001', field='develop.exposure') == 1.25, 20, 0.05)
    timings['a_to_b_ms'] = ms
    check('alices_edit_reaches_bob_live', ms is not None and ms < 5000, ms)
    call('b', op='set', project=pid, key='photo-0002', field='develop.contrast', value=30)
    bobs = lambda r: [x for x in r['pending'] if x['account'] == users['b']['id']]
    ms, rv = wait(lambda: bobs(call('a', op='review', project=pid)), 20, 0.1)
    timings['b_to_a_review_ms'] = ms
    check('bobs_edit_reaches_alice_for_approval', ms is not None, rv)
    ap = call('a', op='approve', project=pid, member=users['b']['id'])
    ms, _ = wait(lambda: call('a', op='get', project=pid, key='photo-0002', field='develop.contrast') == 30
                 and call('b', op='get', project=pid, key='photo-0002', field='develop.contrast') == 30, 20, 0.1)
    check('approved_edit_is_in_both', ms is not None, ap)

    # 5. people by display name
    pa, pb = call('a', op='people', project=pid), call('b', op='people', project=pid)
    names = sorted(x['name'] for x in pa['people'])
    bob = next((x for x in pa['people'] if x['user'] == users['b']['id']), {})
    check('people_by_display_name', names == ['Alice Test', 'Bob Test'] and bob.get('on_project') is True, pa)
    check('no_email_in_any_answer', no_email(pa, pb, lst, call('a', op='status', project=pid), call('b', op='hello')))

    # 6. Bob leaves
    call('b', op='leave', project=pid)
    gone = all(x['id'] != pid for x in call('b', op='hello')['projects'])
    check('leave_takes_it_off_bobs_computer', gone and all(x['project'] != pid for x in call('b', op='shared-projects')['shared']))
    pa = call('a', op='people', project=pid)
    check('alices_computer_takes_him_off_the_log', all(x['user'] != users['b']['id'] for x in pa['people'])
          and users['b']['id'] not in json.dumps(call('a', op='status', project=pid).get('members')), pa)

    # 7. invited again, joins again, removed by Alice: refused
    call('a', op='invite', project=pid, email=users['b']['email'])
    row = next(x for x in call('b', op='shared-projects')['shared'] if x['project'] == pid)
    ok, msg = attempt('b', op='join-shared', shared=row['shared'])
    check('bob_can_join_again_after_leaving', ok, msg)
    if ok:
        call('a', op='remove-person', project=pid, user=users['b']['id'])
        before = call('a', op='get', project=pid, key='photo-0000', field='develop.contrast')
        # his computer may already have the removal (pushed before the refusal): it then refuses
        # the edit itself; either way nothing of his reaches Alice
        ok_set, said = attempt('b', op='set', project=pid, key='photo-0000', field='develop.contrast', value=77)
        time.sleep(8)
        rv = call('a', op='review', project=pid)
        check('removed_member_is_refused', not bobs(rv) and call('a', op='get', project=pid, key='photo-0000', field='develop.contrast') == before,
              {'bob_edit_accepted_locally': ok_set, 'said': said, 'alice_pending': rv['pending']})
        ok2, msg2 = attempt('b', op='people', project=pid)
        check('removed_member_sees_project_not_found', not ok2, msg2)

    # 8. silent renewal on Alice's computer: restart with "renew now", same device, still online
    until0 = call('a', op='hello')['signed_in_until_ms']
    stop('a'); time.sleep(1)
    start('a', {'KOPANO_RENEW_BEFORE_MS': str(31 * 24 * 3600 * 1000)})
    ms, h = wait(lambda: (lambda h: h if h['signed_in'] and h['signed_in_until_ms'] > until0 else None)(call('a', op='hello', timeout=5)), 60, 0.5)
    check('silent_renewal_gives_a_new_certificate', ms is not None and h['device'] == dev_a, {'ms': ms, 'until': h and h['signed_in_until_ms'], 'before': until0})
    stop('a'); start('a')

    # 9. Bob's computer unlinked on the website: next renewal signs it out, with the reason
    dev_b = call('b', op='hello')['device']
    sb('PATCH', f'/rest/v1/kopano_devices?device_key=eq.{dev_b}&revoked_at=is.null',
       {'revoked_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'revoked_reason': 'user'})
    stop('b'); time.sleep(1)
    start('b', {'KOPANO_RENEW_BEFORE_MS': str(31 * 24 * 3600 * 1000)})
    ms, h = wait(lambda: (lambda h: h if not h['signed_in'] and h.get('message') else None)(call('b', op='hello', timeout=5)), 60, 0.5)
    check('unlinked_computer_signs_itself_out_and_says_why', ms is not None and 'unlinked' in h['message'], h)
    ok, msg = attempt('b', op='shared-projects')
    check('unlinked_computer_cannot_use_projects', not ok, msg)

    # Alice signs out and in again: same device, project kept
    call('a', op='sign-out')
    ms, h = wait(lambda: (lambda h: h if not h['signed_in'] else None)(call('a', op='hello', timeout=5)), 30, 0.5)
    check('sign_out_goes_offline', ms is not None, h)
    ok, ms, h = sign_in('a', 'a')
    check('sign_in_again_keeps_device_and_projects', ok and h['device'] == dev_a and any(x['id'] == pid for x in h['projects']), {'ms': ms})
finally:
    for w in list(procs):
        try:
            stop(w)
        except Exception:
            pass
    for u in users.values():
        try:
            sb('DELETE', f'/auth/v1/admin/users/{u["id"]}')
        except Exception as e:
            print('could not delete test account', u['id'], e)
fails = [k for k, v in checks.items() if not v['ok']]
print(json.dumps({'scenario': 's9_signin', 'passed': len(checks) - len(fails), 'total': len(checks), 'failed': fails, 'timings': timings}))
sys.exit(1 if fails else 0)
