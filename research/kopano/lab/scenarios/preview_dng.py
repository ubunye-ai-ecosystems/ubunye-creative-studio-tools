"""Write layer one (the RAW-like preview) as a linear DNG any RAW editor can open.

The pixels are the decoded layer-one data (so the JPEG XL loss is included), linear camera RGB,
with the original's colour matrix, as-shot white balance and orientation, so an editor develops
it the way it develops the original.
"""
import struct, sys, numpy as np, rawpy
import rawpack as R

FLIP_TO_ORIENT = {0: 1, 3: 3, 5: 8, 6: 6}


def _ifd_entries(tags, data_start):
    """tags: list of (tag, type, values). Returns (ifd bytes, extra bytes)."""
    sizes = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 10: 8, 11: 4, 12: 8}
    fmt = {1: 'B', 2: 'B', 3: 'H', 4: 'I', 11: 'f', 12: 'd'}
    entries, extra = [], b''
    n = len(tags)
    extra_off = data_start + 2 + 12 * n + 4
    for tag, typ, vals in sorted(tags, key=lambda t: t[0]):
        if typ in (5, 10):
            raw = b''.join(struct.pack('<ii' if typ == 10 else '<II', *v) for v in vals)
            count = len(vals)
        elif typ == 2:
            raw = vals.encode() + b'\0'; count = len(raw)
        else:
            raw = struct.pack('<' + fmt[typ] * len(vals), *vals); count = len(vals)
        if len(raw) <= 4:
            entries.append(struct.pack('<HHI', tag, typ, count) + raw.ljust(4, b'\0'))
        else:
            entries.append(struct.pack('<HHI', tag, typ, count) + struct.pack('<I', extra_off + len(extra)))
            extra += raw + (b'\0' if len(raw) % 2 else b'')
    return struct.pack('<H', n) + b''.join(entries) + struct.pack('<I', 0), extra


def rational(x, den=1_000_000):
    return (int(round(x * den)), den)


def write_dng(path_in, path_out, lossy=True):
    lin, wb = R.linear_half(path_in)
    ref = R.area_down(lin)
    img = R.preview_decode(R.preview_encode(ref)) if lossy else ref.astype(np.float32)
    img = np.clip(np.round(img), 0, 65535).astype('<u2')
    h, w, _ = img.shape
    with rawpy.imread(path_in) as r:
        cm = np.array(r.rgb_xyz_matrix[:3], np.float64)
        flip = r.sizes.flip
    g = wb[1] or 1.0
    neutral = [g / wb[0], 1.0, g / wb[2]] if wb[0] else [1.0, 1.0, 1.0]
    pix = img.tobytes()
    tags = [
        (254, 4, [0]), (256, 4, [w]), (257, 4, [h]), (258, 3, [16, 16, 16]), (259, 3, [1]),
        (262, 3, [34892]), (271, 2, 'Kopano'), (272, 2, 'layer-one preview'), (273, 4, [0]),
        (274, 3, [FLIP_TO_ORIENT.get(flip, 1)]), (277, 3, [3]), (278, 4, [h]), (279, 4, [len(pix)]),
        (284, 3, [1]), (50706, 1, [1, 4, 0, 0]), (50707, 1, [1, 1, 0, 0]), (50708, 2, 'Kopano preview'),
        (50714, 3, [0, 0, 0]), (50717, 3, [65535, 65535, 65535]),
        (50721, 10, [rational(v) for v in cm.ravel()]), (50728, 5, [rational(v) for v in neutral]),
        (50778, 3, [21]),
    ]
    ifd, extra = _ifd_entries(tags, 8)
    pix_off = 8 + len(ifd) + len(extra)
    tags = [(t, ty, [pix_off] if t == 273 else v) for t, ty, v in tags]
    ifd, extra = _ifd_entries(tags, 8)
    with open(path_out, 'wb') as f:
        f.write(b'II*\0' + struct.pack('<I', 8) + ifd + extra + pix)
    return dict(width=w, height=h, dng_bytes=8 + len(ifd) + len(extra) + len(pix))


if __name__ == '__main__':
    print(write_dng(sys.argv[1], sys.argv[2]))
