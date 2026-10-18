"""E27: kill the process at a random instant during save, sync and media download, many times.
Pass: 0 damaged stores, 0 lost completed edits, every interrupted job completes on restart.
Usage: python e27_crash.py <peer.exe> <work dir> <iterations>"""
import json, os, random, shutil, subprocess, sys, time

P, W, N = sys.argv[1], sys.argv[2], int(sys.argv[3])
random.seed(27)
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '3000'], capture_output=True, check=True)
A, B = f'{W}/alice', f'{W}/bob'
big = f'{W}/clip.bin'
with open(big, 'wb') as f:
    for _ in range(300):
        f.write(os.urandom(1 << 20))
r = subprocess.run([P, 'add-media', '--dir', A, '--file', big], capture_output=True, text=True)
h = json.loads(r.stdout.strip().splitlines()[-1])['hash']
assert len(h) == 64, (r.stdout, r.stderr, h)

def run(args, kill_after=None, timeout=300):
    p = subprocess.Popen([P] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if kill_after is None:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err, False
    try:
        out, err = p.communicate(timeout=kill_after)
        return p.returncode, out, err, False
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
        return None, out, err, True

def healthy(d):
    rc, out, err, _ = run(['verify', '--dir', d])
    rc2, out2, err2, _ = run(['status', '--dir', d])
    ev = [json.loads(l) for l in out.splitlines() if l.startswith('{')]
    clean = rc == 0 and bool(ev) and ev[-1].get('damaged') == []
    return clean and rc2 == 0, (out + err + err2)[-300:]

def value(d, key):
    rc, out, err, _ = run(['status', '--dir', d])
    for line in out.splitlines():
        if f'item {key}:' in line:
            return line
    return None

serve = subprocess.Popen([P, 'serve', '--dir', A], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{A}/addr.json'):
    time.sleep(0.3)
assert run(['pull', '--dir', B, '--from', f'{A}/addr.json'])[0] == 0, 'first pull'
stats = {'edit_killed': 0, 'edit_done': 0, 'pull_killed': 0, 'pull_done': 0, 'media_killed': 0, 'media_done': 0, 'damaged': 0, 'lost_edits': 0, 'restart_failures': 0}
done_edits = {}
problems = []
for i in range(N):
    op = random.choice(['edit', 'edit', 'pull', 'media'])
    delay = random.uniform(0.0, {'edit': 0.5, 'pull': 1.2, 'media': 2.5}[op])
    if op == 'edit':
        # edits go to Bob, so Alice keeps serving undisturbed
        key = f'IMG_{random.randrange(3000):07d}.ARW'
        val = round(random.uniform(-2, 2), 4)
        rc, out, err, killed = run(['edit', '--dir', B, '--key', key, '--field', 'develop.exposure', '--value', str(val)], kill_after=delay)
        if killed:
            stats['edit_killed'] += 1
        elif rc == 0:
            stats['edit_done'] += 1
            done_edits[key] = val
        else:
            problems.append(f'{i} edit failed: {err[-200:]}')
    elif op == 'pull':
        rc, out, err, killed = run(['pull', '--dir', B, '--from', f'{A}/addr.json'], kill_after=delay)
        stats['pull_killed' if killed else 'pull_done'] += 1
        if not killed and rc != 0:
            problems.append(f'{i} pull failed: {err[-200:]}')
    else:
        rc, out, err, killed = run(['fetch-media', '--dir', B, '--from', f'{A}/addr.json', '--hash', h], kill_after=delay)
        stats['media_killed' if killed else 'media_done'] += 1
        if not killed and rc == 0 and random.random() < 0.7:
            shutil.rmtree(f'{B}/media', ignore_errors=True)  # start the next download from zero
        if not killed and rc != 0:
            problems.append(f'{i} media failed: {err[-200:]}')
    ok, why = healthy(B)
    if not ok:
        stats['damaged'] += 1
        problems.append(f'{i} after {op}: store unhealthy: {why}')
        break
# restart everything without kills: all must complete
for args in (['pull', '--dir', B, '--from', f'{A}/addr.json'], ['fetch-media', '--dir', B, '--from', f'{A}/addr.json', '--hash', h]):
    rc, out, err, _ = run(args)
    if rc != 0:
        stats['restart_failures'] += 1
        problems.append(f'final {args[0]} failed: {err[-300:]}')
for key, val in done_edits.items():
    line = value(B, key) or ''
    if f'exposure={val}' not in line and f'exposure={float(val)}' not in line:
        stats['lost_edits'] += 1
        if len(problems) < 8:
            problems.append(f'lost edit {key}={val}: {line}')
serve.kill()
print(json.dumps({'iterations': N, 'stats': stats, 'completed_edits_checked': len(done_edits), 'problems': problems[:8]}))
