"""Side-by-side PNGs with captions for the Film two-window proof (film_proof writes the shots).

Usage: python film_side_by_side.py <out dir>
Reads 1-before-A/B.png, 2-edited-A.png, 3-after-A/B.png and film_proof.json from <out dir>.
"""
import json, os, sys
from PIL import Image, ImageDraw, ImageFont

D = sys.argv[1]
R = json.load(open(os.path.join(D, 'film_proof.json')))


def font(size):
    for f in ['C:/Windows/Fonts/segoeui.ttf', 'C:/Windows/Fonts/arial.ttf']:
        if os.path.exists(f):
            return ImageFont.truetype(f, size)
    return ImageFont.load_default()


def pair(left, right, title, captions, out, width=1400):
    ims = [Image.open(os.path.join(D, p)).convert('RGB') for p in (left, right)]
    w = width // 2
    ims = [im.resize((w, int(im.height * w / im.width))) for im in ims]
    h = max(im.height for im in ims)
    top, line = 64, 30
    bottom = 20 + line * max(len(c) for c in captions)
    canvas = Image.new('RGB', (w * 2 + 12, top + h + bottom), (24, 24, 28))
    d = ImageDraw.Draw(canvas)
    d.text((16, 14), title, fill=(255, 214, 0), font=font(30))
    for i, (im, cap) in enumerate(zip(ims, captions)):
        x = i * (w + 12)
        canvas.paste(im, (x, top))
        for j, c in enumerate(cap):
            d.text((x + 12, top + h + 10 + j * line), c, fill=(235, 235, 235), font=font(21))
    canvas.save(os.path.join(D, out))
    print('wrote', out)


cb, ca = R['B_clip_cut_before'], R['B_clip_cut_after']
tps = 254_016_000_000
sec = lambda v: f'{v / tps:.2f}s'
pair('1-before-A.png', '1-before-B.png', 'Before: one project open in two Ubunye Film windows',
     [['Window A: Thabang (editor)', f'id block {R["idBlocks"][0]}'],
      ['Window B: Mpho (sound)', f'id block {R["idBlocks"][1]} (own ids, never clash)']], 'side-by-side-1-before.png')
pair('2-edited-A.png', '2-edited-B.png', 'Editing at the same time, before anything is shared',
     [['A: first shot trimmed 15 frames at the head,', 'Lumetri grade: exposure +0.9, warmer, more saturated'],
      ['B: first audio clip set to -9 dB (A1 waveform)', 'B still has the old cut and no grade']], 'side-by-side-2-editing.png')
pair('3-after-A.png', '3-after-B.png', 'After Kopano: each window brought in the other\'s version',
     [[f'A now also has B\'s level: {R["A_audio_gain_db"]} dB', f'clashes: {R["clashes"]}'],
      [f'B: cut {sec(cb[1])} long -> {sec(ca[1])}, source from {sec(cb[2])} -> {sec(ca[2])}',
       f'B: grade exposure {R["B_grade_exposure"]}; timelines identical: {R["identical_items"]}',
       f'same frame bytes: {R["frames_identical"]}; same 2 s export bytes: {R.get("export_identical")}']], 'side-by-side-3-after.png')
