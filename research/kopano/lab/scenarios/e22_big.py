"""E22: a 4 GB original over Kopano, the download killed three times, then completed.
Measures resume (bytes already held at each restart), total time and peak memory of the receiver.
Usage: python e22_big.py <peer.exe> <project root with alice> <bob dir (copy target)>"""
import json, os, shutil, subprocess, sys, time

P, ROOT, BOB = sys.argv[1:4]
shutil.rmtree(BOB, ignore_errors=True)
shutil.copytree(os.path.join(ROOT, 'bob'), BOB)
os.makedirs(os.path.join(os.path.dirname(BOB), 'alice'), exist_ok=True)
shutil.copy(os.path.join(ROOT, 'alice', 'cert.json'), os.path.join(os.path.dirname(BOB), 'alice', 'cert.json'))
h = subprocess.run([P, 'hash-of', '--dir', os.path.join(ROOT, 'alice'), '--key', 'big.bin'], capture_output=True, text=True).stdout.strip()
addr = os.path.join(ROOT, 'alice', 'addr.json')
if os.path.exists(addr):
    os.remove(addr)
alice = subprocess.Popen([P, 'serve', '--dir', os.path.join(ROOT, 'alice')], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(addr):
    time.sleep(0.3)

def media_bytes():
    t = 0
    for root, _, files in os.walk(os.path.join(BOB, 'media')):
        for f in files:
            try:
                t += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return t

def peak_mb(pid):
    r = subprocess.run(['powershell', '-NoProfile', '-Command', f'(Get-Process -Id {pid} -ErrorAction SilentlyContinue).WorkingSet64'], capture_output=True, text=True).stdout.strip()
    return int(r) / 1e6 if r.isdigit() else 0

attempts, peak, start = [], 0.0, time.time()
for kill_after in (25, 25, 25, None):
    t0 = time.time()
    held = media_bytes()
    q = subprocess.Popen([P, 'fetch-media', '--dir', BOB, '--from', addr, '--hash', h], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    while q.poll() is None:
        peak = max(peak, peak_mb(q.pid))
        if kill_after and time.time() - t0 > kill_after:
            q.kill()
            break
        time.sleep(1)
    out = q.stdout.read() if q.stdout else ''
    last = [json.loads(l) for l in out.splitlines() if l.startswith('{')]
    attempts.append({'held_at_start_mb': round(held / 1e6), 'secs': round(time.time() - t0, 1), 'killed': kill_after is not None, 'result': last[-1] if last else None})
alice.kill()
done = attempts[-1]['result'] or {}
print(json.dumps({'file_gb': 4.29, 'attempts': attempts, 'total_secs': round(time.time() - start, 1), 'peak_receiver_mb': round(peak), 'complete': done.get('complete') or done.get('already_complete')}))
