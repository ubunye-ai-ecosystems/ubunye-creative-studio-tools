"""E34: many small files (sound effects, stills). One request for the whole folder (a collection)
versus one request per file, on a direct link and on a slow, far link (relay through the chaos
proxy: 4 Mbit/s, 150 ms each way).
Usage: python e34_small_files.py <bin dir> <iroh-relay.exe> <work dir> <files>"""
import json, os, random, shutil, subprocess, sys, time

BIN, RELAY, W, N = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
P, C = os.path.join(BIN, 'peer.exe'), os.path.join(BIN, 'chaos.exe')
random.seed(34)
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '10'], capture_output=True, check=True)
folder = f'{W}/sfx'
os.makedirs(folder)
for i in range(N):
    with open(f'{folder}/sfx_{i:05d}.wav', 'wb') as f:
        f.write(os.urandom(random.randint(2_000, 40_000)))

def ev(r):
    e = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    if not e:
        raise SystemExit(r.stderr[-400:])
    return e[-1]

add = ev(subprocess.run([P, 'add-folder', '--dir', f'{W}/alice', '--folder', folder], capture_output=True, text=True))
h = add['hash']
out = {'files': N, 'mb': round(add['bytes'] / 1e6, 1), 'add_s': round(add['ms'] / 1000, 1), 'runs': []}
relay = subprocess.Popen([RELAY, '--dev'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(2)
for link in ('direct', 'far'):
    env = dict(os.environ)
    chaos = None
    if link == 'far':
        chaos = subprocess.Popen([C, '--listen', '127.0.0.1:4000', '--to', '[::1]:3340', '--kbps', '4000', '--delay-ms', '150', '--drop-every-s', '0'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        env['KOPANO_RELAY_URL'] = 'http://127.0.0.1:4000'
        time.sleep(1)
    extra = ['--relay-only'] if link == 'far' else []
    if os.path.exists(f'{W}/alice/addr.json'):
        os.remove(f'{W}/alice/addr.json')
    a = subprocess.Popen([P, 'serve', '--dir', f'{W}/alice', *extra], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not os.path.exists(f'{W}/alice/addr.json'):
        time.sleep(0.3)
    for mode in ('collection', 'one-by-one'):
        d = f'{W}/bob-{link}-{mode}'
        shutil.copytree(f'{W}/bob', d)
        args = [P, 'fetch-folder', '--dir', d, '--from', f'{W}/alice/addr.json', '--hash', h, *extra] + (['--one-by-one'] if mode == 'one-by-one' else [])
        t = time.time()
        try:
            r = subprocess.run(args, capture_output=True, text=True, env=env, timeout=1800)
            e = ev(r)
            out['runs'].append({'link': link, 'mode': mode, 'requests': e['requests'], 'mb': round(e['bytes'] / 1e6, 1), 'seconds': round(e['ms'] / 1000, 1), 'files_listed': e['files']})
        except subprocess.TimeoutExpired:
            out['runs'].append({'link': link, 'mode': mode, 'seconds': '> 1800 (stopped)'})
        print(json.dumps(out['runs'][-1]), flush=True)
    a.kill()
    if chaos:
        chaos.kill()
relay.kill()
print(json.dumps(out))
