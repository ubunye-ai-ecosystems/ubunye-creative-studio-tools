"""Summarise E12 (pack + progressive preview) into a markdown table. Times are arithmetic from measured bytes."""
import json, os, sys

pack = {json.loads(l)['file']: json.loads(l) for l in open(sys.argv[1])}
prev = {json.loads(l)['file']: json.loads(l) for l in open(sys.argv[2])}
LINKS = (2, 10, 50)  # Mbit/s


def secs(n, mbit):
    return n * 8 / (mbit * 1e6)


def fmt_t(s):
    return f'{s:.1f} s' if s < 60 else f'{s / 60:.1f} min'


rows, tot_orig, tot_pack = [], 0, 0
for f, p in pack.items():
    if 'error' in p:
        rows.append(f'| {os.path.basename(f)} | ERROR {p["error"]} |'); continue
    v = prev.get(f, {})
    size = p['size']
    packed = round(size * p['pct'] / 100)
    tot_orig += size; tot_pack += packed
    pv = v.get('preview_bytes', 0)
    edit = v.get('preview exposure +2')
    jpg = v.get('jpeg exposure +2')
    kind = 'RAW-like' if 'preview exposure +2' in v else ('camera JPEG' if pv else 'none')
    t = ' / '.join(f'{fmt_t(secs(pv, m))}' for m in LINKS)
    t2 = ' / '.join(f'{fmt_t(secs(packed, m))}' for m in LINKS)
    ok = all(x.get('exact', True) for x in p['tried'])
    rows.append(f"| {os.path.basename(f)} | {size / 1e6:.1f} MB | {p['chosen']} | **{p['pct']}%** | {'yes' if ok else 'NO'} "
                f"| {pv / 1e3:.0f} KB {kind} | {edit[1] if edit else '-'}% / {jpg[1] if jpg else '-'}% | {t} | {t2} |")

print('| File | Size | Method | Packed | Exact | Preview (layer 1) | Areas off after +2 EV: preview / JPEG | Layer 1 at 2 / 10 / 50 Mbit/s | Layer 2 at 2 / 10 / 50 Mbit/s |')
print('|---|---|---|---|---|---|---|---|---|')
print('\n'.join(rows))
print(f'\nTotal: {tot_orig / 1e6:.0f} MB -> {tot_pack / 1e6:.0f} MB ({100 * tot_pack / tot_orig:.1f}%) across {len(pack)} files')
