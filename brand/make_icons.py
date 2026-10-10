#!/usr/bin/env python3
"""Generate the Ubunye Creative app icons from the ThabangVision logo motif.

    python -I brand/make_icons.py

The ThabangVision logo is a camera lens with sun rays sitting on a large serif "B".
Each app keeps the lens and rays and swaps the B for its own letter (L, P, F).
The letter is converted to an outline so the SVGs need no installed font.
Font: DM Serif Display (SIL OFL 1.1, brand/fonts/OFL-DMSerifDisplay.txt).
Needs fonttools (pip install fonttools). Writes brand/icons/<app>.svg.
"""
from __future__ import annotations

import math
import pathlib

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

HERE = pathlib.Path(__file__).resolve().parent
BG = "#050505"      # ThabangVision background
LETTER = "#f3ede0"  # warm white, like the logo's B
APPS = {            # letter, accent (lens and rays)
    "light": ("L", "#d4a843"),  # ThabangVision gold
    "photo": ("P", "#e0674a"),  # coral
    "film": ("F", "#3fae9a"),   # teal
}
LENS = (196.0, 196.0)  # lens centre, top left like the logo


def letter_path(font: TTFont, ch: str) -> str:
    glyphs = font.getGlyphSet()
    name = font.getBestCmap()[ord(ch)]
    upm = font["head"].unitsPerEm
    g = glyphs[name]
    # Scale the cap height to ~330 px and sit the letter bottom right, as the B does in the logo.
    scale = 430 / upm
    pen = SVGPathPen(glyphs)
    left = 512 - 66 - g.width * scale * 0.92
    g.draw(TransformPen(pen, (scale, 0, 0, -scale, left, 452)))
    return pen.getCommands()


def lens(accent: str) -> str:
    cx, cy = LENS
    out = [f'<circle cx="{cx}" cy="{cy}" r="104" fill="{BG}"/>']
    # Sun rays fan out over the top and left, alternating long and short like the logo.
    for i, deg in enumerate(range(95, 270, 17)):
        a = math.radians(deg)
        r1, r2 = 118, (176 if i % 2 == 0 else 150)
        x1, y1 = cx + r1 * math.cos(a), cy - r1 * math.sin(a)
        x2, y2 = cx + r2 * math.cos(a), cy - r2 * math.sin(a)
        out.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
                   f'stroke="{accent}" stroke-width="13" stroke-linecap="round"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="90" fill="none" stroke="{accent}" stroke-width="16"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="68" fill="none" stroke="{accent}" stroke-width="7"/>')
    # Aperture blades: six chords tangent to the pupil ring.
    for k in range(6):
        a = math.radians(k * 60 + 15)
        t = (math.cos(a), math.sin(a))
        n = (-t[1], t[0])
        px, py = cx + 30 * n[0], cy + 30 * n[1]
        x1, y1 = px - 58 * t[0], py - 58 * t[1]
        x2, y2 = px + 8 * t[0], py + 8 * t[1]
        out.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
                   f'stroke="{accent}" stroke-width="6" stroke-linecap="round"/>')
    out.append(f'<circle cx="{cx}" cy="{cy}" r="26" fill="{accent}"/>')
    out.append(f'<circle cx="{cx + 9}" cy="{cy - 9}" r="7" fill="{BG}"/>')
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
