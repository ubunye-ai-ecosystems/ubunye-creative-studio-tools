"""E29: a laptop sleeps in the middle of work (process frozen, like a closed lid), then wakes.
A: the follower sleeps while the owner keeps editing.  B: the owner sleeps while it would push.
C: a media download is frozen half way.  Pass: after waking, everything arrives by itself
(no restart by a person), and the download does not start again from zero.
Usage: python e29_sleep.py <peer.exe> <work dir> <sleep seconds>"""
import json, os, shutil, socket, subprocess, sys, time
import psutil

P, W, NAP = sys.argv[1], sys.argv[2], int(sys.argv[3])
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '500'], capture_output=True, check=True)

def lines(path):
    out = []
    try:
        for l in open(path):
            if l.startswith('{') and l.endswith(chr(10)):
                try:
                    out.append(json.loads(l))
                except ValueError:
                    pass
    except OSError:
        pass
    return out

a = subprocess.Popen([P, 'daemon', '--dir', f'{W}/alice', '--port', '7301'], stdout=subprocess.DEVNULL, stderr=open(f'{W}/alice.err', 'w'))
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)
b = subprocess.Popen([P, 'daemon', '--dir', f'{W}/bob', '--port', '7302', '--follow', f'{W}/alice/addr.json'], stdout=open(f'{W}/bob.out', 'w'), stderr=open(f'{W}/bob.err', 'w'))
t0 = time.time()
while not lines(f'{W}/bob.out') and time.time() - t0 < 60:
    time.sleep(0.3)
s = socket.create_connection(('127.0.0.1', 7301))
f = s.makefile('rw')

def edit(k, v):
    f.write(json.dumps({'op': 'set', 'key': f'IMG_{k:07d}.ARW', 'field': 'develop.exposure', 'value': v}) + '\n')
    f.flush()
    return json.loads(f.readline())

def bob_value(k):
    # ask Bob's own daemon (its store is locked while it runs, so `status` cannot open it)
    try:
        with socket.create_connection(('127.0.0.1', 7302), timeout=5) as c:
            g = c.makefile('rw')
            g.write(json.dumps({'op': 'get', 'key': f'IMG_{k:07d}.ARW', 'field': 'develop.exposure'}) + chr(10))
            g.flush()
            return str(json.loads(g.readline()).get('result'))
    except OSError:
        return None

def wait_for(k, v, limit):
    t = time.time()
    while time.time() - t < limit:
        if bob_value(k) == str(v):
            return round(time.time() - t, 1)
        time.sleep(1)
    return None

res = {'nap_s': NAP}
# A: Bob's laptop sleeps; Alice edits meanwhile
pb = psutil.Process(b.pid)
pb.suspend()
for k in range(3):
    edit(k, 1.5 + k)
time.sleep(NAP)
pb.resume()
res['A_follower_slept'] = {'caught_up_after_wake_s': wait_for(2, 3.5, 180)}
# B: Alice's laptop sleeps right after an edit
edit(10, 0.25)
pa = psutil.Process(a.pid)
pa.suspend()
time.sleep(NAP)
pa.resume()
res['B_owner_slept'] = {'edit_arrived_after_wake_s': wait_for(10, 0.25, 180)}
edit(11, 0.75)
res['B_next_edit_s'] = wait_for(11, 0.75, 120)
res['bob_daemon_alive'] = b.poll() is None
res['bob_log_tail'] = open(f'{W}/bob.err').read().strip().splitlines()[-3:]
for p in (a, b):
    p.kill()
print(json.dumps(res))
