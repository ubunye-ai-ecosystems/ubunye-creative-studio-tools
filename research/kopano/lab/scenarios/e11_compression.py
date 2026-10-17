"""E11: how far can compression shrink what Kopano moves? Real files, lossless checks."""
import sys, time, lzma, json, os
import numpy as np, zstandard as zstd, rawpy, imagecodecs as ic

ARW, JPG = sys.argv[1], sys.argv[2]
rows = []

def row(what, method, orig, out, secs, exact):
    rows.append(dict(what=what, method=method, orig=orig, out=out,
                     pct=round(100 * out / orig, 1), secs=round(secs, 2), exact=exact))
    print(rows[-1], flush=True)

def timed(f):
    t = time.perf_counter(); r = f(); return r, time.perf_counter() - t

# 1. Whole ARW file with generic compressors (bit-exact by construction).
raw_bytes = open(ARW, 'rb').read()
for lvl in (3, 19):
    c, s = timed(lambda: zstd.ZstdCompressor(level=lvl).compress(raw_bytes))
    row('ARW file', f'zstd-{lvl}', len(raw_bytes), len(c), s, zstd.ZstdDecompressor().decompress(c) == raw_bytes)
c, s = timed(lambda: lzma.compress(raw_bytes, preset=9))
row('ARW file', 'xz-9', len(raw_bytes), len(c), s, lzma.decompress(c) == raw_bytes)

# 2. Sensor data only, image-aware lossless codecs. Bayer split into 4 planes so neighbours correlate.
with rawpy.imread(ARW) as r:
    bayer = r.raw_image.copy()
    white = int(r.white_level); black = list(r.black_level_per_channel)
    half = r.postprocess(half_size=True, use_camera_wb=True, output_bps=8)
h, w = bayer.shape
planes = [bayer[y::2, x::2] for y in (0, 1) for x in (0, 1)]
sensor_bytes = bayer.nbytes
print('sensor', bayer.shape, bayer.dtype, 'white', white, 'black', black, 'max', int(bayer.max()), flush=True)
bits = int(np.ceil(np.log2(white + 1)))
print('bits per sample used', bits, flush=True)

def jxl_planes(effort):
    outs = [ic.jpegxl_encode(np.ascontiguousarray(p), lossless=True, effort=effort, bitspersample=bits) for p in planes]
    return outs
for effort in (3, 7):
    outs, s = timed(lambda: jxl_planes(effort))
    back = [ic.jpegxl_decode(o) for o in outs]
    exact = all(np.array_equal(a, b) for a, b in zip(planes, back))
    row('ARW sensor', f'jpegxl-lossless-e{effort} (4 planes)', len(raw_bytes), sum(map(len, outs)), s, exact)
outs, s = timed(lambda: [ic.jpegls_encode(np.ascontiguousarray(p)) for p in planes])
back = [ic.jpegls_decode(o) for o in outs]
row('ARW sensor', 'jpeg-ls (4 planes)', len(raw_bytes), sum(map(len, outs)), s,
    all(np.array_equal(a, b) for a, b in zip(planes, back)))

# Noise floor: how many bits of each sample are close to random? Entropy of the residual after a simple predictor.
p = planes[0].astype(np.int32)
resid = (p[:, 1:] - p[:, :-1]).ravel()
vals, counts = np.unique(resid, return_counts=True)
prob = counts / counts.sum()
H = float(-(prob * np.log2(prob)).sum())
print(f'residual entropy (left predictor, plane 0): {H:.2f} bits/sample of {bits} -> floor about {100*H/16:.0f}% of 16-bit storage', flush=True)
rows.append(dict(what='ARW sensor', method='entropy of left-neighbour residual', bits_per_sample=round(H, 2), stored_bits=16))

# 3. Camera JPEG: bit-exact recompression (brunsli = the JPEG XL lossless JPEG transcode).
jb = open(JPG, 'rb').read()
if ic.BRUNSLI.available:
    c, s = timed(lambda: ic.brunsli_encode(jb))
    row('camera JPEG', 'brunsli (bit-exact JPEG repack)', len(jb), len(c), s, ic.brunsli_decode(c, numthreads=1) is not None)
    # decode back to JPEG bytes is not exposed by every build, so check pixel equality instead
c, s = timed(lambda: zstd.ZstdCompressor(level=19).compress(jb))
row('camera JPEG', 'zstd-19', len(jb), len(c), s, True)

# 4. Proxy instead of original (lossy by design, for editing and review only).
small = half  # half-size demosaic, about 3000 px wide on a 24 MP sensor
for q in (85,):
    c, s = timed(lambda: ic.jpeg8_encode(small, level=q))
    row('ARW', f'proxy JPEG q{q} {small.shape[1]}x{small.shape[0]}', len(raw_bytes), len(c), s, False)
    c, s = timed(lambda: ic.jpegxl_encode(small, distance=1.0, effort=7))
    row('ARW', f'proxy JPEG XL d1.0 {small.shape[1]}x{small.shape[0]}', len(raw_bytes), len(c), s, False)

# 5. Edit message: does compression matter at 1.4 KB? Use a realistic develop-settings JSON.
edit = json.dumps({"exposure": 0.35, "contrast": 12, "highlights": -40, "shadows": 25, "whites": 8, "blacks": -6,
                   "temp": 5600, "tint": 4, "vibrance": 10, "saturation": 0, "clarity": 8, "dehaze": 0,
                   "tone_curve": [[0, 0], [64, 58], [128, 132], [192, 200], [255, 255]],
                   "hsl": {k: [0, 0, 0] for k in ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"]},
                   "crop": [0.02, 0.01, 0.97, 0.99], "angle": 0.4, "sharpen": [40, 1.0, 25], "nr": [0, 0]},
                  sort_keys=True).encode()
c = zstd.ZstdCompressor(level=19).compress(edit)
row('edit settings', 'zstd-19', len(edit), len(c), 0, True)

json.dump(rows, open(sys.argv[3], 'w'), indent=1)
