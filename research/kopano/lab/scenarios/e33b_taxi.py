"""E33b: the data taxi. Alice seals a day's work (edits + a 24 MB original) for the project's
members. Carol, who is NOT a member, carries it (a taxi driver's phone). Pass: Carol cannot open
or read it, any change she makes is refused, and Bob imports it intact.
Usage: python e33b_taxi.py <peer.exe> <work dir>"""
import json, os, shutil, subprocess, sys

P, W = sys.argv[1:3]
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '2000', '--extra-peers', '1'], capture_output=True, check=True)
A, B, C, M = (f'{W}/{n}' for n in ('alice', 'bob', 'carol', 'm0'))

def run(*args):
    r = subprocess.run([P, *args], capture_output=True, text=True, timeout=600)
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    return r.returncode, (ev[-1] if ev else None), r.stderr.strip()[-160:]

for i in range(30):
    run('edit', '--dir', A, '--key', f'IMG_{i:07d}.ARW', '--field', 'develop.exposure', '--value', '0.42')
raw = os.urandom(24 << 20)
open(f'{W}/raw.ARW', 'wb').write(raw)
h = run('add-media', '--dir', A, '--file', f'{W}/raw.ARW')[1]['hash']
run('export-bundle', '--dir', A, '--out', f'{W}/day.kpb', '--media', h)
_, ev, _ = run('seal-bundle', '--file', f'{W}/day.kpb', '--to', f'{B}/cert.json,{M}/cert.json', '--out', f'{W}/day.kps')
os.remove(f'{W}/day.kpb')  # only the sealed copy leaves Alice
sealed = open(f'{W}/day.kps', 'rb').read()
out = {'sealed_mb': round(len(sealed) / 1e6, 1), 'recipients': ev['recipients']}
out['carrier_sees_original_bytes'] = raw[:64] in sealed or raw[len(raw) // 2:len(raw) // 2 + 64] in sealed
out['carrier_sees_project_name'] = b'Wedding' in sealed
rc, ev, err = run('import-bundle', '--dir', C, '--file', f'{W}/day.kps')
out['carrier_can_open'] = rc == 0
out['carrier_error'] = err
bad = bytearray(sealed)
bad[len(bad) // 3] ^= 0x20
open(f'{W}/day-tampered.kps', 'wb').write(bad)
def head(d):
    return open(f'{d}/head').read() if os.path.exists(f'{d}/head') else None

head_before = head(B)
rc, ev, err = run('import-bundle', '--dir', B, '--file', f'{W}/day-tampered.kps')
out['tampered_refused'] = rc != 0 and head(B) == head_before
out['tampered_error'] = err
rc, ev, err = run('import-bundle', '--dir', B, '--file', f'{W}/day.kps')
out['bob_import'] = ev
out['bob_equals_alice'] = head(A) == head(B)
print(json.dumps(out))
