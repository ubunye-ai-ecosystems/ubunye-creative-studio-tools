"""E28: the disk fills up during a media download and during a big sync.
Bob's folder lives on a small (200 MB) disk. Pass: a clear error, no damaged store, and the job
finishes once space is freed (download resumes, not restarts).
Usage: python e28_disk_full.py <peer.exe> <work dir on a big disk> <bob dir on the small disk>"""
import json, os, shutil, subprocess, sys, time

P, W, BOBDIR = sys.argv[1:4]
shutil.rmtree(W, ignore_errors=True)
shutil.rmtree(BOBDIR, ignore_errors=True)
small = os.path.splitdrive(BOBDIR)[0] + '/'
subprocess.run([P, 'setup', '--root', W, '--synthetic', '20000'], capture_output=True, check=True)
shutil.copytree(f'{W}/bob', BOBDIR)
os.makedirs(os.path.join(os.path.dirname(BOBDIR), 'alice'), exist_ok=True)
shutil.copy(f'{W}/alice/cert.json', os.path.join(os.path.dirname(BOBDIR), 'alice', 'cert.json'))
big = f'{W}/clip.bin'
with open(big, 'wb') as f:
    for _ in range(150):
        f.write(os.urandom(1 << 20))
h = json.loads(subprocess.run([P, 'add-media', '--dir', f'{W}/alice', '--file', big], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash']
serve = subprocess.Popen([P, 'serve', '--dir', f'{W}/alice'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)

def free_mb():
    return round(shutil.disk_usage(small).free / 1e6, 1)

def run(args):
    r = subprocess.run([P] + args, capture_output=True, text=True, timeout=600)
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    return r.returncode, ev[-1] if ev else None, r.stderr.strip().splitlines()[-1][:200] if r.stderr.strip() else ''

def healthy():
    rc, ev, err = run(['verify', '--dir', BOBDIR])
    return rc == 0 and ev is not None and ev.get('damaged') == [], err

out = {}
# leave about 60 MB free, then ask for a 157 MB original
filler = os.path.join(small, 'filler.bin')
with open(filler, 'wb') as f:
    f.write(b'\0' * int((shutil.disk_usage(small).free - 60e6)))
out['free_before_mb'] = free_mb()
rc, ev, err = run(['fetch-media', '--dir', BOBDIR, '--from', f'{W}/alice/addr.json', '--hash', h])
out['download_when_full'] = {'exit': rc, 'error': err, 'event': ev}
out['healthy_after_full_download'] = healthy()
out['free_after_mb'] = free_mb()
# a big sync (20,000 items) with almost nothing free
with open(os.path.join(small, 'filler2.bin'), 'wb') as f:
    f.write(b'\0' * max(0, int(shutil.disk_usage(small).free - 2e6)))
out['free_before_sync_mb'] = free_mb()
rc, ev, err = run(['pull', '--dir', BOBDIR, '--from', f'{W}/alice/addr.json'])
out['sync_when_full'] = {'exit': rc, 'error': err, 'event': ev and {k: ev.get(k) for k in ('objects', 'result')}}
out['healthy_after_full_sync'] = healthy()
# free space: both must now finish, the download resuming
os.remove(filler)
os.remove(os.path.join(small, 'filler2.bin'))
out['free_after_cleanup_mb'] = free_mb()
rc, ev, err = run(['pull', '--dir', BOBDIR, '--from', f'{W}/alice/addr.json'])
out['sync_after_space'] = {'exit': rc, 'error': err, 'result': ev and ev.get('result')}
rc, ev, err = run(['fetch-media', '--dir', BOBDIR, '--from', f'{W}/alice/addr.json', '--hash', h])
out['download_after_space'] = {'exit': rc, 'error': err, 'had_before': ev and ev.get('had_before'), 'received': ev and ev.get('bytes_received'), 'complete': ev and (ev.get('complete') or ev.get('already_complete'))}
out['healthy_at_end'] = healthy()
serve.kill()
print(json.dumps(out))
