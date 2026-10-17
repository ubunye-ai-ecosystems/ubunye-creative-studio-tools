"""Compare Light renders: original RAW vs Light smart preview (original offline), same edit. Writes contact sheets."""
import sys, os, json, numpy as np, imagecodecs as ic
from e13_light_compare import load, to, blocks_off
W = sys.argv[1]
for stem in sys.argv[2:]:
    d = f'{W}/{stem}/out'
    for ev in ('-2', '0', '2'):
        a, b = load(f'{d}/original.ev{ev}.png'), load(f'{d}/smart.ev{ev}.png')
        h, w = min(a.shape[0], b.shape[0]) // 2, min(a.shape[1], b.shape[1]) // 2
        ra, rb = to(a, h, w), to(b, h, w)
        print(json.dumps(dict(file=stem, exposure=ev, smart_vs_original=blocks_off(ra, rb), same_size=a.shape == b.shape)))
        if ev != '0':
            s = 3
            sheet = np.concatenate([ra[::s, ::s], np.full((ra[::s, ::s].shape[0], 6, 3), 255.0), rb[::s, ::s]], 1).astype(np.uint8)
            open(f'{W}/{stem}.sheet.ev{ev}.png', 'wb').write(ic.png_encode(sheet))
