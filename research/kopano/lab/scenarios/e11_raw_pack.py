import sys, time, numpy as np, rawpy, imagecodecs as ic
f = open(sys.argv[1], 'rb').read()
with rawpy.imread(sys.argv[1]) as r:
    b = r.raw_image.copy()
sens = b.astype('<u2').tobytes()
off = f.find(sens[:4096])
print('sensor offset', off, 'sensor bytes', len(sens), 'file', len(f))
assert off >= 0 and f[off:off+len(sens)] == sens, 'sensor not stored contiguously'
head, tail = f[:off], f[off+len(sens):]
t = time.perf_counter()
planes = [np.ascontiguousarray(b[y::2, x::2]) for y in (0,1) for x in (0,1)]
packed = [ic.jpegls_encode(p) for p in planes]
import zstandard as z
hz = z.ZstdCompressor(level=19).compress(head + tail)
enc = time.perf_counter() - t
t = time.perf_counter()
back = [ic.jpegls_decode(p) for p in packed]
s = np.empty_like(b)
for (y, x), p in zip([(0,0),(0,1),(1,0),(1,1)], back): s[y::2, x::2] = p
ht = z.ZstdDecompressor().decompress(hz)
rebuilt = ht[:off] + s.astype('<u2').tobytes() + ht[off:]
dec = time.perf_counter() - t
total = sum(map(len, packed)) + len(hz)
print(f'pack total {total} = {100*total/len(f):.1f}% of file; other bytes {len(head)+len(tail)} -> {len(hz)}; encode {enc:.2f}s decode {dec:.2f}s; byte-identical {rebuilt == f}')
