"""Kopano RAW pack + progressive original (E12 prototype, research only).

pack(file) tries methods from best to plainest and keeps the smallest one whose
unpack gives back the original file byte for byte. "store" always works, so no
camera can fail; cameras only differ in how much they save.

  sensor  find the sensor samples inside the file (16-bit LE/BE, or bit-packed
          10/12/14-bit, MSB or LSB first, any row stride), compress them with
          JPEG-LS per 2x2 colour plane, zstd the rest of the file.
  lj92    precomp-style: every lossless-JPEG stream (Canon CR2, most DNGs) is
          decoded and re-encoded by `lj92` (Rust) to prove the camera's exact
          bytes come back; the samples then go to JPEG-LS, the rest to zstd.
  generic any file: embedded JPEG previews repacked by JPEG XL (byte-exact),
          everything else zstd level 19 with long matching.

All three methods use the generic layer for their non-pixel bytes.
  store   the file as it is.

preview(file) makes the first layer of a progressive original: linear camera
RGB (no white balance, no clipping, no tone curve) at <= 2560 px long edge,
lossy JPEG XL. Edits are made on it like on the RAW; layer two is the pack.
"""
import io, json, os, struct, subprocess, sys, tempfile, time, hashlib
import numpy as np, rawpy, zstandard as zstd, imagecodecs as ic

MAGIC = b'KRP1'


# ---------- bit layouts ----------
def encode_rows(rows, lay):
    """rows: (n, w) uint16 -> bytes exactly as the layout stores them (no stride padding)."""
    kind = lay['kind']
    if kind == 'le16':
        return rows.astype('<u2').tobytes()
    if kind == 'be16':
        return rows.astype('>u2').tobytes()
    b, order = lay['bits'], lay['order']
    v = rows.astype(np.uint32).reshape(rows.shape[0], -1)
    if order == 'msb':
        bits = ((v[..., None] >> np.arange(b - 1, -1, -1, dtype=np.uint32)) & 1).astype(np.uint8)
        return np.packbits(bits.reshape(rows.shape[0], -1), axis=1).tobytes()
    bits = ((v[..., None] >> np.arange(b, dtype=np.uint32)) & 1).astype(np.uint8)
    return np.packbits(bits.reshape(rows.shape[0], -1), axis=1, bitorder='little').tobytes()


def row_bytes(w, lay):
    if lay['kind'] in ('le16', 'be16'):
        return 2 * w
    return (w * lay['bits'] + 7) // 8


LAYOUTS = [dict(kind='le16'), dict(kind='be16')] + [
    dict(kind='packed', bits=b, order=o) for b in (12, 14, 10) for o in ('msb', 'lsb')]


def find_sensor(f, raw):
    """Return (layout, offset, stride) such that rows of `raw` sit in f, or None."""
    h, w = raw.shape
    probe_row = h // 2
    for lay in LAYOUTS:
        if lay['kind'] == 'packed' and int(raw.max()) >= (1 << lay['bits']):
            continue
        rb = row_bytes(w, lay)
        if rb * h > len(f):
            continue
        seg = encode_rows(raw[probe_row:probe_row + 2], lay)
        a = f.find(seg[:rb])
        while a >= 0:
            b = f.find(seg[rb:], a + rb)
            if b >= 0 and b - a < 2 * rb:
                stride = b - a
                off = a - probe_row * stride
                if off >= 0 and off + (h - 1) * stride + rb <= len(f):
                    return lay, off, stride
            a = f.find(seg[:rb], a + 1)
    return None


def verify_rows(f, raw, lay, off, stride, chunk=128):
    h, w = raw.shape
    rb = row_bytes(w, lay)
    for y in range(0, h, chunk):
        enc = encode_rows(raw[y:y + chunk], lay)
        n = min(chunk, h - y)
        for i in range(n):
            s = off + (y + i) * stride
            if f[s:s + rb] != enc[i * rb:(i + 1) * rb]:
                return False
    return True


def planes_of(a):
    return [np.ascontiguousarray(a[y::2, x::2]) for y in (0, 1) for x in (0, 1)]


def unplanes(planes, h, w):
    out = np.empty((h, w), np.uint16)
    for (y, x), p in zip([(0, 0), (0, 1), (1, 0), (1, 1)], planes):
        out[y::2, x::2] = p
    return out


# ---------- container ----------
def _frame(meta, blobs):
    m = json.dumps(meta, sort_keys=True).encode()
    out = io.BytesIO()
    out.write(MAGIC + struct.pack('<I', len(m)) + m + struct.pack('<I', len(blobs)))
    for b in blobs:
        out.write(struct.pack('<Q', len(b)) + b)
    return out.getvalue()


def _unframe(data):
    assert data[:4] == MAGIC
    n = struct.unpack_from('<I', data, 4)[0]
    meta = json.loads(data[8:8 + n])
    p = 8 + n
    k = struct.unpack_from('<I', data, p)[0]; p += 4
    blobs = []
    for _ in range(k):
        L = struct.unpack_from('<Q', data, p)[0]; p += 8
        blobs.append(data[p:p + L]); p += L
    return meta, blobs


# ---------- the non-pixel bytes: embedded JPEG previews + everything else ----------
def _jpeg_end(b, soi):
    """End (exclusive) of a Huffman JPEG starting at soi, or -1. Lossless (SOF3) streams are skipped."""
    pos, n, sof = soi + 2, len(b), None
    while pos + 4 <= n:
        if b[pos] != 0xFF:
            return -1
        m = b[pos + 1]
        if m == 0xFF:
            pos += 1; continue
        if m == 0xD9:
            return pos + 2 if sof is not None else -1
        if 0xD0 <= m <= 0xD7 or m == 0x01:
            pos += 2; continue
        L = (b[pos + 2] << 8) | b[pos + 3]
        if m in (0xC0, 0xC1, 0xC2):
            sof = m
        elif 0xC3 <= m <= 0xCF and m not in (0xC4, 0xC8, 0xCC):
            return -1
        pos += 2 + L
        if m == 0xDA:
            if sof is None:
                return -1
            while True:  # skip entropy-coded data
                pos = b.find(b'\xff', pos)
                if pos < 0 or pos + 1 >= n:
                    return -1
                nx = b[pos + 1]
                if nx == 0x00 or 0xD0 <= nx <= 0xD7 or nx == 0xFF:
                    pos += 1 if nx == 0xFF else 2; continue
                break
    return -1


def pack_rest(rest):
    """Every embedded Huffman JPEG that JPEG XL can repack byte-exactly goes to JPEG XL; the rest to zstd."""
    rest = bytes(rest)
    keep, jpegs, blobs, cur, i = [], [], [], 0, 0
    while True:
        i = rest.find(b'\xff\xd8\xff', i)
        if i < 0:
            break
        e = _jpeg_end(rest, i)
        if e < 0 or e - i < 4096:
            i += 2; continue
        seg = rest[i:e]
        try:
            j = ic.jpegxl_encode_jpeg(seg)
            ok = len(j) < len(seg) and ic.jpegxl_decode_jpeg(j) == seg
        except Exception:
            ok = False
        if ok:
            keep.append(rest[cur:i]); jpegs.append([sum(map(len, keep)), len(seg)]); blobs.append(j); cur = e
        i = e
    keep.append(rest[cur:])
    stripped = b''.join(keep)
    params = zstd.ZstdCompressionParameters.from_level(19, enable_ldm=True, window_log=27)
    return _frame(dict(jpegs=jpegs, size=len(rest)), [zstd.ZstdCompressor(compression_params=params).compress(stripped)] + blobs)


def unpack_rest(data):
    meta, blobs = _unframe(data)
    stripped = zstd.ZstdDecompressor().decompress(blobs[0], max_output_size=meta['size'])
    out, cur = [], 0
    for (pos, n), j in zip(meta['jpegs'], blobs[1:]):
        out.append(stripped[cur:pos]); out.append(ic.jpegxl_decode_jpeg(j)); cur = pos
    out.append(stripped[cur:])
    return b''.join(out)


def pack_sensor(f, path):
    try:
        with rawpy.imread(path) as r:
            raw = r.raw_image.copy()
    except Exception as e:
        return None, f'libraw: {type(e).__name__}'
    if raw.ndim != 2:
        return None, 'not a single-plane sensor image'
    hit = find_sensor(f, raw)
    if not hit:
        return None, 'sensor samples not stored plainly (compressed in camera)'
    lay, off, stride = hit
    if not verify_rows(f, raw, lay, off, stride):
        return None, 'layout matched a probe but not every row'
    h, w = raw.shape
    rb = row_bytes(w, lay)
    # everything that is not sensor bytes, in file order
    rest = bytearray(f[:off])
    for y in range(h - 1):
        s = off + y * stride
        rest += f[s + rb:s + stride]
    rest += f[off + (h - 1) * stride + rb:]
    blobs = [pack_rest(rest)]
    blobs += [ic.jpegls_encode(p) for p in planes_of(raw)]
    meta = dict(method='sensor', layout=lay, off=off, stride=stride, h=h, w=w, size=len(f))
    return _frame(meta, blobs), None


LJ92_BIN = os.environ.get('KOPANO_LJ92', 'T:/rust/targets/kopano/release/lj92.exe')


def _arrangements(a, nf):
    """Ways to lay out a sample block for JPEG-LS: name -> (list of 2D arrays)."""
    out = {'plain': [a], 'quad': planes_of(a)}
    if nf > 1:
        out['comp'] = [np.ascontiguousarray(a[:, c::nf]) for c in range(nf)]
        out['comp_rows'] = [np.ascontiguousarray(a[y::2, c::nf]) for c in range(nf) for y in (0, 1)]
    return out


def _unarrange(name, parts, h, w, nf, slices=None):
    if name == 'plain':
        return parts[0]
    if name == 'quad':
        return unplanes(parts, h, w)
    if name == 'cr2_quad':
        n, sw, last = slices
        rw = n * sw + last
        return reslice(unplanes(parts, h * w // rw, rw), slices).reshape(h, w)
    a = np.empty((h, w), np.uint16)
    if name == 'comp':
        for c in range(nf):
            a[:, c::nf] = parts[c]
        return a
    i = 0
    for c in range(nf):
        for y in (0, 1):
            a[y::2, c::nf] = parts[i]; i += 1
    return a


def cr2_slices(f):
    """Canon CR2 tag 0xC640 (n, width, last width): the sensor image is cut into vertical strips."""
    i = f.find(bytes.fromhex('40c6030003000000'))  # tag C640, type SHORT, count 3, little-endian
    if i < 0:
        return None
    off = struct.unpack_from('<I', f, i + 8)[0]
    n, w, last = struct.unpack_from('<3H', f, off)
    return (n, w, last) if w and last else None


def unslice(stream, sl):
    n, w, last = sl
    widths = [w] * n + [last]
    rw = sum(widths)
    h = stream.size // rw
    img = np.empty((h, rw), np.uint16)
    p = x = 0
    for sw in widths:
        img[:, x:x + sw] = stream[p:p + h * sw].reshape(h, sw)
        p += h * sw; x += sw
    return img


def reslice(img, sl):
    n, w, last = sl
    out, x = [], 0
    for sw in [w] * n + [last]:
        out.append(img[:, x:x + sw].ravel()); x += sw
    return np.concatenate(out)


def _lj92_groups(meta):
    """Streams with the same width and component count are stacked into one tall image (DNG tiles)."""
    groups = {}
    for i, st in enumerate(meta['streams']):
        groups.setdefault((st['width'], st['components']), []).append(i)
    return [dict(width=w, nf=nf, members=m) for (w, nf), m in groups.items()]


def pack_lj92(f, path):
    with tempfile.TemporaryDirectory() as td:
        rest_p, smp_p, meta_p = (os.path.join(td, n) for n in ('rest', 'smp', 'meta'))
        r = subprocess.run([LJ92_BIN, 'strip', path, rest_p, smp_p, meta_p], capture_output=True, text=True)
        if r.returncode != 0:
            return None, f'lj92 strip failed: {r.stderr.strip()[:80]}'
        meta = json.load(open(meta_p))
        if not meta['streams']:
            return None, 'no lossless-JPEG streams'
        rest = open(rest_p, 'rb').read()
        smp = np.fromfile(smp_p, dtype='<u2')
    blobs = [pack_rest(rest)]
    groups = _lj92_groups(meta)
    for g in groups:
        sts = [meta['streams'][i] for i in g['members']]
        a = np.concatenate([smp[st['sample_offset']:st['sample_offset'] + st['samples']].reshape(st['height'], st['width'])
                            for st in sts])
        best = None
        cands = _arrangements(a, g['nf'])
        sl = cr2_slices(f) if len(sts) == 1 else None
        if sl and a.size % (sl[0] * sl[1] + sl[2]) == 0:
            g['slices'] = list(sl)
            cands['cr2_quad'] = planes_of(unslice(a.ravel(), sl))
        for name, parts in cands.items():
            enc = [ic.jpegls_encode(p) for p in parts]
            n = sum(map(len, enc))
            if best is None or n < best[0]:
                best = (n, name, enc)
        g['arrangement'], g['parts'], g['height'] = best[1], len(best[2]), int(a.shape[0])
        blobs += best[2]
    return _frame(dict(method='lj92', size=len(f), lj92=meta, groups=groups), blobs), None


def unpack_lj92(meta, blobs):
    rest = unpack_rest(blobs[0])
    lm = meta['lj92']
    smp = np.empty(sum(st['samples'] for st in lm['streams']), np.uint16)
    k = 1
    for g in meta['groups']:
        parts = [ic.jpegls_decode(b) for b in blobs[k:k + g['parts']]]; k += g['parts']
        a = _unarrange(g['arrangement'], parts, g['height'], g['width'], g['nf'], g.get('slices'))
        y = 0
        for i in g['members']:
            st = lm['streams'][i]
            smp[st['sample_offset']:st['sample_offset'] + st['samples']] = a[y:y + st['height']].ravel()
            y += st['height']
    with tempfile.TemporaryDirectory() as td:
        rest_p, smp_p, meta_p, out_p = (os.path.join(td, n) for n in ('rest', 'smp', 'meta', 'out'))
        open(rest_p, 'wb').write(rest)
        smp.astype('<u2').tofile(smp_p)
        json.dump(lm, open(meta_p, 'w'))
        r = subprocess.run([LJ92_BIN, 'restore', rest_p, smp_p, meta_p, out_p], capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(r.stderr)
        return open(out_p, 'rb').read()


def pack_generic(f):
    """Any file at all: embedded JPEGs repacked, everything else zstd."""
    return _frame(dict(method='generic', size=len(f)), [pack_rest(f)])


def pack_store(f):
    return _frame(dict(method='store', size=len(f)), [f])


def unpack(data):
    meta, blobs = _unframe(data)
    m = meta['method']
    if m == 'store':
        return blobs[0]
    if m == 'generic':
        return unpack_rest(blobs[0])
    if m == 'lj92':
        return unpack_lj92(meta, blobs)
    rest = unpack_rest(blobs[0])
    h, w, lay, off, stride = meta['h'], meta['w'], meta['layout'], meta['off'], meta['stride']
    raw = unplanes([ic.jpegls_decode(b) for b in blobs[1:]], h, w)
    rb = row_bytes(w, lay)
    gap = stride - rb
    out = bytearray(rest[:off])
    p = off
    enc_all = []
    for y in range(0, h, 256):
        enc_all.append(encode_rows(raw[y:y + 256], lay))
    enc = b''.join(enc_all)
    for y in range(h):
        out += enc[y * rb:(y + 1) * rb]
        if y < h - 1:
            out += rest[p:p + gap]; p += gap
    out += rest[p:]
    return bytes(out)


def pack(path):
    """Return (best_container, report). Every candidate is unpacked and compared."""
    f = open(path, 'rb').read()
    report = dict(size=len(f), tried=[])
    best = pack_store(f)
    best_name = 'store'
    for name, fn in (('sensor', lambda: pack_sensor(f, path)), ('lj92', lambda: pack_lj92(f, path)),
                     ('generic', lambda: (pack_generic(f), None))):
        t = time.perf_counter()
        c, why = fn()
        tp = time.perf_counter() - t
        if c is None:
            report['tried'].append(dict(method=name, skipped=why)); continue
        t = time.perf_counter()
        ok = unpack(c) == f
        tu = time.perf_counter() - t
        report['tried'].append(dict(method=name, pct=round(100 * len(c) / len(f), 1), pack_s=round(tp, 2),
                                    unpack_s=round(tu, 2), exact=ok))
        if ok and len(c) < len(best):
            best, best_name = c, name
    report['chosen'] = best_name
    report['pct'] = round(100 * len(best) / len(f), 1)
    report['sha256'] = hashlib.sha256(f).hexdigest()
    return best, report


# ---------- progressive original, layer one ----------
LONG_EDGE = 2560
GAMMA = 2.2


def linear_half(path):
    """Linear camera RGB, 16-bit, no WB, no auto-bright, half size (quads merged, no demosaic)."""
    with rawpy.imread(path) as r:
        img = r.postprocess(half_size=True, gamma=(1, 1), no_auto_bright=True, output_bps=16,
                            user_wb=[1.0, 1.0, 1.0, 1.0], output_color=rawpy.ColorSpace.raw,
                            user_flip=0)
        wb = list(r.camera_whitebalance)
    return img, wb


def _area_matrix(n_in, n_out):
    """Row-stochastic matrix that averages input samples into n_out output samples (area resampling)."""
    edges = np.linspace(0, n_in, n_out + 1)
    m = np.zeros((n_out, n_in), np.float32)
    for i in range(n_out):
        lo, hi = edges[i], edges[i + 1]
        for j in range(int(lo), min(int(np.ceil(hi)), n_in)):
            m[i, j] = min(hi, j + 1) - max(lo, j)
        m[i] /= m[i].sum()
    return m


def area_down(img, long_edge=LONG_EDGE):
    h, w = img.shape[:2]
    s = long_edge / max(h, w)
    if s >= 1:
        return img
    ho, wo = max(1, round(h * s)), max(1, round(w * s))
    mh, mw = _area_matrix(h, ho), _area_matrix(w, wo)
    x = img.astype(np.float32)
    out = np.stack([mh @ x[..., c] @ mw.T for c in range(x.shape[2])], axis=-1)
    return np.round(np.clip(out, 0, 65535)).astype(np.uint16)


CURVES = {
    # value v in [0,1] linear -> code in [0,1]
    'gamma': (lambda v: v ** (1 / 2.2), lambda c: c ** 2.2),
    # log curve: 16 stops below white get equal code steps (like camera log profiles)
    'log': (lambda v: np.clip(np.log2(np.maximum(v, 2 ** -16)) / 16 + 1, 0, 1),
            lambda c: np.where(c <= 0, 0, 2 ** ((c - 1) * 16))),
}


def preview_encode(lin16, distance=0.7, curve='gamma'):
    enc = CURVES[curve][0]
    g = np.round(65535.0 * enc(lin16.astype(np.float32) / 65535.0)).astype(np.uint16)
    return ic.jpegxl_encode(g, distance=distance, effort=7, bitspersample=16)


def preview_decode(data, curve='gamma'):
    dec = CURVES[curve][1]
    return dec(ic.jpegxl_decode(data).astype(np.float32) / 65535.0) * 65535.0


def wb_mul(wb):
    g = wb[1] if wb[1] else 1.0
    return np.array([wb[0] / g, 1.0, wb[2] / g], np.float32) if wb[0] else np.ones(3, np.float32)


EDITS = {  # name: (exposure EV, extra white-balance multipliers)
    'as shot': (0, (1, 1, 1)),
    'exposure -2': (-2, (1, 1, 1)),
    'exposure +2': (2, (1, 1, 1)),
    'warmer': (0, (1.15, 1, 0.85)),
}


def render(lin, wb, edit):
    """A neutral stand-in for any editor: white balance, exposure, clip, display gamma, 8-bit."""
    ev, extra = EDITS[edit]
    x = lin.astype(np.float32) / 65535.0 * wb_mul(wb) * np.array(extra, np.float32) * (2.0 ** ev)
    return np.round(255 * np.clip(x, 0, 1) ** (1 / 2.2)).astype(np.uint8)


def compare(a, b, block=8):
    """(PSNR dB, % of 8x8 areas whose average colour is off by more than 2 levels of 255)."""
    mse = float(((a.astype(np.float32) - b.astype(np.float32)) ** 2).mean())
    psnr = 99.0 if mse == 0 else 10 * np.log10(255 ** 2 / mse)
    h, w = a.shape[0] // block * block, a.shape[1] // block * block
    def means(x):
        return x[:h, :w].astype(np.float32).reshape(h // block, block, w // block, block, 3).mean(axis=(1, 3))
    d = np.abs(means(a) - means(b)).max(axis=-1)
    return round(psnr, 1), round(100 * float((d > 2).mean()), 2)


def embedded_preview(f):
    """Largest Huffman JPEG the camera stored inside the file (view-only fallback layer one)."""
    best, i = b'', 0
    while True:
        i = f.find(b'\xff\xd8\xff', i)
        if i < 0:
            return best
        e = _jpeg_end(f, i)
        if e > 0 and e - i > len(best):
            best = f[i:e]
        i = e if e > 0 else i + 2


def preview_test(path, distance=0.7, curve='gamma'):
    try:
        lin, wb = linear_half(path)
    except Exception as e:
        jp = embedded_preview(open(path, 'rb').read())
        if not jp:
            return dict(preview=None, preview_bytes=0, reason=f'{type(e).__name__}; no embedded JPEG either')
        img = ic.jpeg8_decode(jp)
        small = area_down(img.astype(np.uint16) * 257) // 257
        pv = ic.jpegxl_encode(small.astype(np.uint8), distance=1.0, effort=7)
        return dict(preview='camera JPEG, resized (view only, RAW decoder does not read this format)',
                    preview_bytes=len(pv), preview_px=f'{small.shape[1]}x{small.shape[0]}',
                    embedded_jpeg_bytes=len(jp), reason=type(e).__name__)
    ref = area_down(lin)
    t = time.perf_counter(); pv = preview_encode(ref, distance, curve); te = time.perf_counter() - t
    dec = preview_decode(pv, curve)
    # the usual shortcut: an 8-bit display JPEG made once, edited afterwards
    jpg = ic.jpeg8_encode(render(ref, wb, 'as shot'), level=90)
    jd = ic.jpeg8_decode(jpg).astype(np.float32) / 255.0
    jlin = (jd ** 2.2) / wb_mul(wb) * 65535.0  # undo display gamma and WB so the same edit applies
    out = dict(preview_bytes=len(pv), preview_px=f'{ref.shape[1]}x{ref.shape[0]}', preview_s=round(te, 2),
               jpeg_bytes=len(jpg), distance=distance, curve=curve)
    for e in EDITS:
        r0 = render(ref, wb, e)
        out[f'preview {e}'] = compare(r0, render(dec, wb, e))
        out[f'jpeg {e}'] = compare(r0, render(jlin, wb, e))
    return out


if __name__ == '__main__':
    mode, files = sys.argv[1], sys.argv[2:]
    for p in files:
        try:
            rep = pack(p)[1] if mode == 'pack' else preview_test(p, *[float(x) if x[0].isdigit() else x for x in mode.split(':')[1:]])
        except Exception as e:
            rep = dict(error=f'{type(e).__name__}: {e}')
        print(json.dumps(dict(file=p, **rep)), flush=True)
