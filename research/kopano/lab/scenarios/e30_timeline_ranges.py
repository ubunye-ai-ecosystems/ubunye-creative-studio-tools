"""E30: use parts of a remote video original without downloading it.
On import, the owner's device builds a small timeline index (container header ranges + byte range
of every packet). A teammate's editor turns timeline cuts into byte ranges, fetches only those
(each 1 KiB chunk verified by BLAKE3/bao against the original's hash), and decodes the cuts from a
sparse copy. Pass: decoded frames identical to decoding the full original; bytes moved
proportional to seconds used.
Usage: python e30_timeline_ranges.py <peer.exe> <clip.mp4> <work dir>"""
import json, os, shutil, struct, subprocess, sys, time

P, CLIP, W = sys.argv[1:4]
FF = shutil.which('ffmpeg'); FP = shutil.which('ffprobe')
shutil.rmtree(W, ignore_errors=True)
os.makedirs(W)

def boxes(path):
    """Top-level MP4 boxes: (type, start, end)."""
    out, size = [], os.path.getsize(path)
    with open(path, 'rb') as f:
        at = 0
        while at < size:
            f.seek(at)
            n, t = struct.unpack('>I4s', f.read(8))
            if n == 1:
                n = struct.unpack('>Q', f.read(8))[0]
            elif n == 0:
                n = size - at
            out.append((t.decode('latin1'), at, at + n))
            at += n
    return out

# 1. owner side, at import: the timeline index (travels with the project, small)
t0 = time.time()
pk = json.loads(subprocess.run([FP, '-v', 'error', '-show_entries', 'packet=stream_index,pts_time,pos,size,flags', '-of', 'json', CLIP], capture_output=True, text=True).stdout)['packets']
packets = [(p['stream_index'], float(p['pts_time']), int(p['pos']), int(p['size']), 'K' in p.get('flags', '')) for p in pk if p.get('pos') not in (None, 'N/A') and p.get('pts_time') not in (None, 'N/A')]
header = [(s, e) if t != 'mdat' else (s, s + 16) for t, s, e in boxes(CLIP)]  # every box, mdat only its 16-byte header
index = {'header': header, 'keyframes': sorted(t for si, t, *_ , k in packets if si == 0 and k), 'packets': packets}
index_bytes = len(json.dumps({'header': header, 'packets': [(si, round(t, 3), pos, sz) for si, t, pos, sz, _ in packets]}).encode())
index_s = round(time.time() - t0, 1)
duration = max(t for _, t, *_ in packets)

def ranges_for(cuts):
    """Byte ranges for timeline cuts, widened back to the keyframe before each cut."""
    rs, starts = list(header), []
    for a, b in cuts:
        kf = max([k for k in index['keyframes'] if k <= a] or [0.0])
        starts.append((kf, b))
        for si, t, pos, sz, _ in packets:
            if kf - 0.05 <= t <= b + 0.05:
                rs.append((pos, pos + sz))
    rs.sort()
    merged = []
    for s, e in rs:
        if merged and s <= merged[-1][1] + 4096:
            merged[-1] = (merged[-1][0], max(merged[-1][1], e))
        else:
            merged.append((s, e))
    return merged, starts

def framemd5(path, a, b):
    r = subprocess.run([FF, '-v', 'error', '-ss', f'{a:.3f}', '-to', f'{b:.3f}', '-i', path, '-map', '0:v:0', '-f', 'framemd5', '-'], capture_output=True, text=True)
    return [l.split(',')[-1].strip() for l in r.stdout.splitlines() if not l.startswith('#')], r.stderr.strip()[-200:]

# 2. two devices, the owner holds the original
subprocess.run([P, 'setup', '--root', W, '--synthetic', '10'], capture_output=True, check=True)
r = subprocess.run([P, 'add-media', '--dir', f'{W}/alice', '--file', CLIP], capture_output=True, text=True)
h = json.loads(r.stdout.strip().splitlines()[-1])['hash']
serve = subprocess.Popen([P, 'serve', '--dir', f'{W}/alice'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)

size = os.path.getsize(CLIP)
rows = []
sparse = f'{W}/bob-sparse.mp4'
plans = [
    ('one 5 s cut', [(10.0, 15.0)]),
    ('three cuts, 12 s', [(20.0, 24.0), (40.0, 44.0), (60.0, 64.0)]),
    ('add 2 s next to a held cut', [(15.0, 17.0)]),
]
for name, cuts in plans:
    rs, starts = ranges_for(cuts)
    spec = ','.join(f'{s}-{e}' for s, e in rs)
    t = time.time()
    out = subprocess.run([P, 'fetch-ranges', '--dir', f'{W}/bob', '--from', f'{W}/alice/addr.json', '--hash', h, '--ranges', spec, '--out', sparse], capture_output=True, text=True)
    ev = [json.loads(l) for l in out.stdout.splitlines() if l.startswith('{')]
    secs = round(time.time() - t, 2)
    same = []
    for (kf, b) in starts:
        got, err = framemd5(sparse, kf, b)
        want, _ = framemd5(CLIP, kf, b)
        same.append({'from_s': round(kf, 2), 'to_s': b, 'frames': len(want), 'identical': got == want and len(want) > 0, 'decode_err': err if got != want else ''})
    rows.append({'plan': name, 'seconds_used': sum(b - a for a, b in cuts), 'ranges': len(rs), 'received_bytes': ev[-1]['bytes_received'] if ev else None, 'fetch_s': secs, 'cuts': same, 'error': out.stderr[-300:] if not ev else ''})
serve.kill()
print(json.dumps({'clip_mb': round(size / 1e6, 1), 'duration_s': round(duration, 1), 'index_kb': round(index_bytes / 1024, 1), 'index_build_s': index_s, 'runs': rows}))
