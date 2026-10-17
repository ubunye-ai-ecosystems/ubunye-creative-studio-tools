"""E13 step 2: compare Light renders of the original RAW, the preview DNG and the JPEG proxy (same edit)."""
import sys, os, json, numpy as np, imagecodecs as ic
import rawpack as R

def load(p):
    a = ic.png_decode(open(p, 'rb').read())[..., :3]
    return a

def to(a, h, w):
    m_h, m_w = R._area_matrix(a.shape[0], h), R._area_matrix(a.shape[1], w)
    x = a.astype(np.float32)
    return np.stack([m_h @ x[..., c] @ m_w.T for c in range(3)], -1)

def blocks_off(a, b, k=16, thr=2.0):
    H, W = a.shape[0] // k * k, a.shape[1] // k * k
    m = lambda x: x[:H, :W].reshape(H // k, k, W // k, k, 3).mean(axis=(1, 3))
    d = np.abs(m(a) - m(b)).max(-1)
    return round(100 * float((d > thr).mean()), 2), round(float(np.abs(m(a) - m(b)).mean()), 2)

def main():
  d, names = sys.argv[1], sys.argv[2:]
  out = []
  for n in names:
    for ev in ('-2', '0', '2'):
        raw = load(f'{d}/{n}.raw.ev{ev}.png'); h, w = raw.shape[0] // 2, raw.shape[1] // 2
        r = to(raw, h, w)
        pv = to(load(f'{d}/{n}.pv.ev{ev}.png'), h, w)
        jp = to(load(f'{d}/{n}.jpg.ev{ev}.png'), h, w)
        row = dict(file=n, exposure=ev, preview_vs_raw=blocks_off(r, pv), jpeg_vs_raw=blocks_off(r, jp))
        out.append(row); print(json.dumps(row))
        if ev != '0':
            s = 3
            sheet = np.concatenate([x[::s, ::s].astype(np.uint8) for x in (r, pv, jp)], axis=1)
            open(f'{d}/{n}.sheet.ev{ev}.png', 'wb').write(ic.png_encode(sheet))


if __name__ == "__main__":
    main()
