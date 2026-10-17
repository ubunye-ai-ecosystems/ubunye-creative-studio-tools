"""E13 step 1: render the same edit on the full RAW, on the RAW-like preview and on a JPEG proxy, side by side."""
import sys, os, numpy as np, imagecodecs as ic
import rawpack as R


def label(img, text):
    # tiny 5x7 bitmap font would be overkill; mark panels with a coloured bar instead
    bar = np.zeros((14, img.shape[1], 3), np.uint8)
    colours = {'RAW': (255, 255, 255), 'preview': (80, 200, 120), 'jpeg': (230, 80, 80)}
    bar[:] = colours[text]
    return np.concatenate([bar, img])


def brightest_crop(img, h=360, w=480):
    lum = img.astype(np.float32).mean(axis=2)
    k = 32
    H, W = lum.shape[0] // k * k, lum.shape[1] // k * k
    blocks = lum[:H, :W].reshape(H // k, k, W // k, k).mean(axis=(1, 3))
    y, x = np.unravel_index(np.argmax(blocks), blocks.shape)
    cy, cx = y * k + k // 2, x * k + k // 2
    y0 = int(np.clip(cy - h // 2, 0, img.shape[0] - h)); x0 = int(np.clip(cx - w // 2, 0, img.shape[1] - w))
    return (y0, x0, h, w)


def run(path, out_dir):
    lin, wb = R.linear_half(path)
    ref = R.area_down(lin)
    dec = R.preview_decode(R.preview_encode(ref))
    jpg = ic.jpeg8_encode(R.render(ref, wb, 'as shot'), level=90)
    jd = ic.jpeg8_decode(jpg).astype(np.float32) / 255.0
    jlin = (jd ** 2.2) / R.wb_mul(wb) * 65535.0
    name = os.path.splitext(os.path.basename(path))[0]
    for edit in ('exposure -2', 'exposure +2'):
        panels = {k: R.render(v, wb, edit) for k, v in (('RAW', ref), ('preview', dec), ('jpeg', jlin))}
        # whole frame, shrunk to 640 wide
        s = max(1, panels['RAW'].shape[1] // 640)
        row1 = np.concatenate([label(p[::s, ::s], k) for k, p in panels.items()], axis=1)
        # 100% crop of the brightest area (where clipped JPEG data shows first)
        y0, x0, h, w = brightest_crop(R.render(ref, wb, 'as shot'))
        row2 = np.concatenate([label(p[y0:y0 + h, x0:x0 + w], k) for k, p in panels.items()], axis=1)
        if row2.shape[1] < row1.shape[1]:
            row2 = np.pad(row2, ((0, 0), (0, row1.shape[1] - row2.shape[1]), (0, 0)))
        else:
            row1 = np.pad(row1, ((0, 0), (0, row2.shape[1] - row1.shape[1]), (0, 0)))
        sheet = np.concatenate([row1, np.full((8, row1.shape[1], 3), 40, np.uint8), row2])
        fn = os.path.join(out_dir, f"{name}_{edit.replace(' ', '')}.png")
        open(fn, 'wb').write(ic.png_encode(sheet))
        print(fn, sheet.shape)


if __name__ == '__main__':
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for p in sys.argv[2:]:
        run(p, out)
