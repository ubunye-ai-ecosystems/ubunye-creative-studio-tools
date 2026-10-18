"""E32 + E33: no network at all, and people never online at the same time.
E32: Alice's day of work (edits + two originals) goes to Bob on a "USB stick" (a bundle file);
then only what changed since; a damaged bundle and another project's bundle must change nothing.
E33: Alice and Bob are never online together; Mpho (a member) syncs with Alice, Alice goes
offline, Bob syncs from Mpho.
Usage: python e32_offline.py <peer.exe> <work dir>"""
import json, os, random, shutil, subprocess, sys, time

P, W = sys.argv[1:3]
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '2000', '--extra-peers', '1'], capture_output=True, check=True)
A, B, M = f'{W}/alice', f'{W}/bob', f'{W}/m0'
STICK = f'{W}/stick'
os.makedirs(STICK)

def run(*args, ok=True):
    r = subprocess.run([P, *args], capture_output=True, text=True, timeout=600)
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    if ok and r.returncode != 0:
        raise SystemExit(f'{args[0]} failed: {r.stderr[-300:]}')
    return r.returncode, (ev[-1] if ev else None), r.stderr.strip()[-200:]

def head(d):
    return open(f'{d}/head').read().strip()

out = {}
# Alice's day: 50 edits and two originals
for i in range(50):
    run('edit', '--dir', A, '--key', f'IMG_{i:07d}.ARW', '--field', 'develop.exposure', '--value', str(round(0.01 * i, 2)))
media = []
for name, mb in (('raw.ARW', 24), ('clip.MP4', 100)):
    with open(f'{W}/{name}', 'wb') as f:
        f.write(os.urandom(mb << 20))
    media.append(run('add-media', '--dir', A, '--file', f'{W}/{name}')[1]['hash'])
# E32a: the whole day on a stick
_, ev, _ = run('export-bundle', '--dir', A, '--out', f'{STICK}/day1.kpb', '--media', *media)
out['day1_bundle'] = ev
_, ev, _ = run('import-bundle', '--dir', B, '--file', f'{STICK}/day1.kpb')
out['day1_import'] = ev
out['day1_same_head'] = head(A) == head(B)
# E32b: next day, only what changed (Bob leaves his head on the stick)
bob_had = head(B)
for i in range(5):
    run('edit', '--dir', A, '--key', f'IMG_{100 + i:07d}.ARW', '--field', 'develop.exposure', '--value', '0.5')
_, ev, _ = run('export-bundle', '--dir', A, '--have', bob_had, '--out', f'{STICK}/day2.kpb')
out['day2_bundle_bytes'] = ev['file_bytes']
# E32c: a damaged copy and another project's bundle change nothing
raw = bytearray(open(f'{STICK}/day2.kpb', 'rb').read())
raw[len(raw) // 2] ^= 0xFF
open(f'{STICK}/day2-damaged.kpb', 'wb').write(raw)
rc, ev, err = run('import-bundle', '--dir', B, '--file', f'{STICK}/day2-damaged.kpb', ok=False)
out['damaged_refused'] = rc != 0 and head(B) == bob_had
out['damaged_error'] = err
other = f'{W}/other'
subprocess.run([P, 'setup', '--root', other, '--synthetic', '10'], capture_output=True, check=True)
run('export-bundle', '--dir', f'{other}/alice', '--out', f'{STICK}/other.kpb')
rc, ev, err = run('import-bundle', '--dir', B, '--file', f'{STICK}/other.kpb', ok=False)
out['other_project_refused'] = rc != 0 and head(B) == bob_had
_, ev, _ = run('import-bundle', '--dir', B, '--file', f'{STICK}/day2.kpb')
out['day2_import'] = ev and ev['result']
out['day2_same_head'] = head(A) == head(B)
# a damaged original inside a bundle is rejected, the rest still imports
run('export-bundle', '--dir', A, '--out', f'{STICK}/media.kpb', '--media', media[0])
raw = bytearray(open(f'{STICK}/media.kpb', 'rb').read())
raw[-1000] ^= 0x01
open(f'{STICK}/media-damaged.kpb', 'wb').write(raw)
fresh = f'{W}/bob2'
shutil.copytree(f'{W}/bob.orig' if os.path.exists(f'{W}/bob.orig') else B, fresh)
shutil.rmtree(f'{fresh}/media', ignore_errors=True)
_, ev, _ = run('import-bundle', '--dir', fresh, '--file', f'{STICK}/media-damaged.kpb')
out['damaged_original'] = {'media_ok': ev['media_ok'], 'media_rejected': ev['media_rejected']}
# E33: never online together; Mpho carries
for i in range(7):
    run('edit', '--dir', A, '--key', f'IMG_{200 + i:07d}.ARW', '--field', 'develop.exposure', '--value', '-0.3')
alice = subprocess.Popen([P, 'serve', '--dir', A], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{A}/addr.json'):
    time.sleep(0.3)
run('pull', '--dir', M, '--from', f'{A}/addr.json')
alice.kill()
alice.wait()
os.remove(f'{A}/addr.json')
mpho = subprocess.Popen([P, 'serve', '--dir', M], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{M}/addr.json'):
    time.sleep(0.3)
_, ev, _ = run('pull', '--dir', B, '--from', f'{M}/addr.json')
mpho.kill()
out['E33_carried'] = {'result': ev and ev['result'], 'bob_equals_alice': head(A) == head(B)}
print(json.dumps(out))
