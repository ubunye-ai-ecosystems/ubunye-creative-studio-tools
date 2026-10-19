"""Side-by-side sheets for the Ubunye Photo connector proof (photo_proof).

Usage: python photo_side_by_side.py <out dir>
Reads <out>/a/<step>.png, <out>/b/<step>.png and <out>/measurements.json and writes
<out>/side-by-side/<step>.png: window A (Alice) left, window B (Bob) right, a caption on top.
"""

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

CAPTIONS = {
    "0-start": "Both windows open the same photo (the owner's DSC08379, 3000x2000). Bob starts from Alice's first version.",
    "1a-new-exposure-layer": "Alice adds an Exposure adjustment layer (+0.3). It reaches Bob through Kopano.",
    "1b-exposure-change": "Alice moves the Exposure slider to +0.7. Only that one setting travels.",
    "2a-new-retouch-layer": "Alice adds an empty Retouch layer.",
    "2b-one-brush-stroke": "Alice paints one brush stroke. Only the tiles it touched travel.",
    "3a-both-painted-before-sync": "At the same time: Alice paints blue (top left), Bob paints pink (top right), same layer.",
    "3b-merged": "Each brings in the other's work: different tiles, no clash, both strokes kept in both windows.",
}


def font(size):
    for name in ["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"]:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


def numbers(row):
    if not row:
        return ""
    keys = [("items_changed", "items changed"), ("tile_items_changed", "tiles changed"), ("tile_items_in_document", "tiles in doc"),
            ("kopano_bytes", "version bytes"), ("media_raw_bytes", "media bytes raw"), ("media_zstd_bytes", "media bytes zstd")]
    parts = [f"{label}: {row[k]:,}" for k, label in keys if k in row]
    return "   ".join(parts)


def main(out):
    out = Path(out)
    rows = {r["step"]: r for r in json.loads((out / "measurements.json").read_text())}
    dest = out / "side-by-side"
    dest.mkdir(exist_ok=True)
    big, small = font(26), font(20)
    for a in sorted((out / "a").glob("*.png")):
        b = out / "b" / a.name
        if not b.exists():
            continue
        step = a.stem
        ia, ib = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
        h = max(ia.height, ib.height)
        gap, top = 16, 110
        sheet = Image.new("RGB", (ia.width + ib.width + gap * 3, h + top + gap), (24, 24, 24))
        sheet.paste(ia, (gap, top))
        sheet.paste(ib, (gap * 2 + ia.width, top))
        d = ImageDraw.Draw(sheet)
        d.text((gap, 12), CAPTIONS.get(step, step), font=big, fill=(240, 240, 240))
        d.text((gap, 50), numbers(rows.get(step)), font=small, fill=(255, 210, 63))
        d.text((gap, 80), "Window A: Alice", font=small, fill=(180, 180, 180))
        d.text((gap * 2 + ia.width, 80), "Window B: Bob", font=small, fill=(180, 180, 180))
        sheet.save(dest / f"{step}.png")
        print(dest / f"{step}.png")


if __name__ == "__main__":
    main(sys.argv[1])
