#!/usr/bin/env python3
"""Generate the Ubunye Creative app icons from the ThabangVision logo motif.

    python -I brand/make_icons.py

The ThabangVision logo is a camera lens with sun rays sitting on a large serif "B".
Each app keeps the lens and rays and swaps the B for its own mark: a capital U for Ubunye
plus the app's letter (Ul, Up, Uf). The letters are converted to outlines so the SVGs need no
installed font.
Font: DM Serif Display (SIL OFL 1.1, brand/fonts/OFL-DMSerifDisplay.txt).
Needs fonttools (pip install fonttools). Writes brand/icons/<app>.svg.
"""
from __future__ import annotations

import math
import pathlib

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

HERE = pathlib.Path(__file__).resolve().parent
BG = "#050505"      # ThabangVision background
LETTER = "#f3ede0"  # warm white, like the logo's B
APPS = {            # letter, accent (lens and rays)
    "light": ("Ul", "#d4a843"),  # ThabangVision gold
    "photo": ("Up", "#e0674a"),  # coral
    "film": ("Uf", "#3fae9a"),   # teal
}
LENS = (168.0, 168.0)
K = 0.8  # lens and rays scale  # lens centre, top left like the logo


def letter_path(font: TTFont, text: str) -> str:
    """Outline `text` and fit its real outline into the bottom right box, as the B sits in the logo."""
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    names = [cmap[ord(c)] for c in text]
    # Measure the outline of the whole string in font units.
    bounds = BoundsPen(glyphs)
    x = 0
    for n in names:
        glyphs[n].draw(TransformPen(bounds, (1, 0, 0, 1, x, 0)))
        x += glyphs[n].width
    xmin, ymin, xmax, ymax = bounds.bounds
    box_w, box_h, right, bottom = 340, 330, 512 - 52, 512 - 48
    scale = min(box_w / (xmax - xmin), box_h / (ymax - ymin))
    ox = right - xmax * scale
    oy = bottom + ymin * scale  # y is flipped: font baseline goes to oy
    pen = SVGPathPen(glyphs)
    x = 0
    for n in names:
        glyphs[n].draw(TransformPen(pen, (scale, 0, 0, -scale, ox + x * scale, oy)))
        x += glyphs[n].width
    return pen.getCommands()


def lens(accent: str) -> str:
    cx, cy = LENS
    out = [f'<circle cx="{cx}" cy="{cy}" r="{104*K:.0f}" fill="{BG}"/>']
    # Sun rays fan out over the top and left, alternating long and short like the logo.
    for i, deg in enumerate(range(95, 270, 17)):
        a = math.radians(deg)
        r1, r2 = 118 * K, (176 if i % 2 == 0 else 150) * K
        x1, y1 = cx + r1 * math.cos(a), cy - r1 * math.sin(a)
        x2, y2 = cx + r2 * math.cos(a), cy - r2 * math.sin(a)
        out.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
                   f'stroke="{accent}" stroke-width="{13*K:.0f}" stroke-linecap="round"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="{90*K:.0f}" fill="none" stroke="{accent}" stroke-width="{16*K:.0f}"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="{68*K:.0f}" fill="none" stroke="{accent}" stroke-width="{7*K:.0f}"/>')
    # Aperture blades: six chords tangent to the pupil ring.
    for k in range(6):
        a = math.radians(k * 60 + 15)
        t = (math.cos(a), math.sin(a))
        n = (-t[1], t[0])
        px, py = cx + 30 * K * n[0], cy + 30 * K * n[1]
        x1, y1 = px - 58 * K * t[0], py - 58 * K * t[1]
        x2, y2 = px + 8 * K * t[0], py + 8 * K * t[1]
        out.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
                   f'stroke="{accent}" stroke-width="{6*K:.0f}" stroke-linecap="round"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="{26*K:.0f}" fill="{accent}"/>')
    out.append(f'<circle cx="{cx + 9*K:.0f}" cy="{cy - 9*K:.0f}" r="{7*K:.0f}" fill="{BG}"/>')
    return "\n  ".join(out)


def main() -> None:
    font = TTFont(HERE / "fonts" / "DMSerifDisplay-Regular.ttf")
    (HERE / "icons").mkdir(exist_ok=True)
    for app, (ch, accent) in APPS.items():
        svg = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">\n'
            f"  <!-- Ubunye Creative {app.title()} icon, adapted from the ThabangVision logo. "
            "Ubunye AI Ecosystems artwork, MIT OR Apache-2.0. Letter outline from DM Serif Display (SIL OFL 1.1). -->\n"
            f'  <rect width="512" height="512" rx="112" fill="{BG}"/>\n'
            f'  <path d="{letter_path(font, ch)}" fill="{LETTER}"/>\n'
            f"  {lens(accent)}\n"
            "</svg>\n"
        )
        (HERE / "icons" / f"{app}.svg").write_text(svg, encoding="utf-8", newline="\n")
        print("wrote", app)


if __name__ == "__main__":
    main()
