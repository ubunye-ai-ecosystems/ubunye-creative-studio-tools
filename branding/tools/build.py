#!/usr/bin/env python3
"""Build every Ubunye Creative Studio brand asset from the design tokens.

    python branding/tools/build.py            # SVG masters + all raster exports
    python branding/tools/build.py --svg-only # SVG masters only

Needs: fonttools and Pillow (pip install fonttools pillow), and resvg on PATH or in
RESVG (cargo install resvg). All geometry is drawn here as vector paths; no raster
reference image is embedded anywhere. Letters are converted to outlines, so the
SVGs need no installed font.

Outputs (relative to branding/):
  icons/<app>/<app>-icon.svg            master icon (1024 canvas, glow border)
  icons/<app>/<app>-icon-small.svg      simplified icon for 16 to 32 px
  icons/<app>/<app>-icon-macos.svg      master on Apple's 824/1024 grid
  icons/<app>/png/<app>-<size>.png      16 ... 1024
  icons/<app>/<app>.ico, <app>.icns, hicolor/<size>/apps/<app id>.png
  wordmarks/ubunye-creative-studio-{dark,light}.svg (+ png)
  logos/<app>/<app>-{horizontal,vertical}-{dark,light}.svg (+ png)
  design-tokens/tokens.{css,ts,rs}
"""
from __future__ import annotations

import json
import math
import os
import pathlib
import shutil
import subprocess
import sys

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOK = json.loads((ROOT / "design-tokens" / "tokens.json").read_text(encoding="utf-8"))
C = TOK["color"]
APPS = TOK["apps"]
APP_IDS = {"light": "org.ubunye.creative.light", "photo": "org.ubunye.creative.photo", "film": "org.ubunye.creative.film"}
RESVG = os.environ.get("RESVG") or shutil.which("resvg") or "T:/rust/cargo/bin/resvg.exe"

DISPLAY = TTFont(ROOT / "fonts" / "DMSerifDisplay-Regular.ttf")
_inter = TTFont(ROOT / "fonts" / "Inter[opsz,wght].ttf")
LABEL = instancer.instantiateVariableFont(_inter, {"wght": 500, "opsz": 14})


# ---------------------------------------------------------------- text outlines
def text_path(font: TTFont, text: str, tracking: float = 0.0) -> tuple[str, tuple[float, float, float, float], float]:
    """Return (svg path at 1 unit = 1 font unit, ink bounds, advance width)."""
    gs = font.getGlyphSet()
    cmap = font.getBestCmap()
    upm = font["head"].unitsPerEm
    pen, bounds, x = SVGPathPen(gs), BoundsPen(gs), 0.0
    for ch in text:
        if ch == " ":
            x += gs[cmap[32]].width + tracking * upm
            continue
        g = gs[cmap[ord(ch)]]
        g.draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        g.draw(TransformPen(bounds, (1, 0, 0, -1, x, 0)))
        x += g.width + tracking * upm
    return pen.getCommands(), bounds.bounds, x - tracking * upm


def cap_units(font: TTFont) -> float:
    """Measured cap height (ink top of H), more reliable than the OS/2 table."""
    gs = font.getGlyphSet()
    b = BoundsPen(gs)
    gs[font.getBestCmap()[ord("H")]].draw(b)
    return b.bounds[3]


def place(font: TTFont, text: str, *, x: float, baseline: float, cap: float, fill: str,
          tracking: float = 0.0, anchor: str = "left", condense: float = 1.0) -> tuple[str, float]:
    """Outline `text` with cap height `cap` px, return (svg element, rendered width)."""
    path, _, adv = text_path(font, text, tracking)
    capu = cap_units(font)
    s = cap / capu
    w = adv * s * condense
    if anchor == "middle":
        x -= w / 2
    elif anchor == "right":
        x -= w
    return f'<path transform="translate({x:.2f} {baseline:.2f}) scale({s * condense:.5f} {s:.5f})" d="{path}" fill="{fill}"/>', w


# ---------------------------------------------------------------- icon geometry
# All values are fractions of the tile side, measured from the reference board.
LENS_C = (0.420, 0.392)
RING_R, RING_W = 0.172, 0.040      # thick outer ring
INNER_R, INNER_W = 0.132, 0.011    # thin inner ring
PUPIL_R = 0.040
MASK_R = 0.200                     # dark disc behind the lens, hides the top of the U
RAY_IN = 0.232
RAY_LONG, RAY_SHORT = 0.345, 0.300
RAY_W = 0.029
RAY_ANGLES = [97, 113, 129, 145, 162, 180, 197, 214, 231]  # degrees, counter clockwise from +x
MONO_LEFT, MONO_BASELINE, MONO_CAP = 0.270, 0.838, 0.440
MONO_TRACK = -0.075
MONO_CONDENSE = 0.86  # the reference monogram is tightly kerned and slightly condensed


def lens(t: float, ox: float, oy: float, accent: str, small: bool) -> str:
    cx, cy = ox + LENS_C[0] * t, oy + LENS_C[1] * t
    p = []
    p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{MASK_R * t:.1f}" fill="{C["bg"]}"/>')
    angles = RAY_ANGLES if not small else [97, 125, 153, 180, 207, 233]
    w = RAY_W * t * (1.9 if small else 1.0)
    for i, deg in enumerate(angles):
        a = math.radians(deg)
        r2 = (RAY_LONG if i % 2 == 0 else RAY_SHORT) * t
        r1 = RAY_IN * t
        if small:
            r1, r2 = 0.235 * t, 0.33 * t
        p.append(f'<line x1="{cx + r1 * math.cos(a):.1f}" y1="{cy - r1 * math.sin(a):.1f}" '
                 f'x2="{cx + r2 * math.cos(a):.1f}" y2="{cy - r2 * math.sin(a):.1f}" '
                 f'stroke="{accent}" stroke-width="{w:.1f}" stroke-linecap="round"/>')
    ring_w = RING_W * t * (1.6 if small else 1.0)
    p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{RING_R * t:.1f}" fill="none" stroke="{accent}" stroke-width="{ring_w:.1f}"/>')
    if small:
        p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{PUPIL_R * 1.9 * t:.1f}" fill="{accent}"/>')
        return "\n    ".join(p)
    p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{INNER_R * t:.1f}" fill="none" stroke="{accent}" stroke-width="{INNER_W * t:.1f}"/>')
    # Six aperture blades: each is a chord from the inner ring to the pupil, rotated 60 degrees,
    # which reads as the swirled iris of the reference.
    for k in range(6):
        a0 = math.radians(k * 60 + 20)
        sx, sy = cx + INNER_R * t * math.cos(a0), cy + INNER_R * t * math.sin(a0)
        a1 = a0 + math.radians(118)
        ex, ey = cx + PUPIL_R * 1.35 * t * math.cos(a1), cy + PUPIL_R * 1.35 * t * math.sin(a1)
        p.append(f'<line x1="{sx:.1f}" y1="{sy:.1f}" x2="{ex:.1f}" y2="{ey:.1f}" '
                 f'stroke="{accent}" stroke-width="{0.0085 * t:.1f}" stroke-linecap="round"/>')
    p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{PUPIL_R * 1.35 * t:.1f}" fill="none" stroke="{accent}" stroke-width="{0.0085 * t:.1f}"/>')
    p.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{PUPIL_R * t:.1f}" fill="{accent}"/>')
    p.append(f'<circle cx="{cx - 0.010 * t:.1f}" cy="{cy - 0.012 * t:.1f}" r="{0.011 * t:.1f}" fill="{C["bg"]}"/>')
    return "\n    ".join(p)


def icon_svg(app: str, *, size: int = 1024, margin: float = 0.055, small: bool = False, glow: bool = True) -> str:
    accent = C[app]
    t = size * (1 - 2 * margin)
    o = size * margin
    rx = TOK["radius"]["icon-ratio"] * t
    mono = APPS[app]["monogram"]
    letters, _ = place(DISPLAY, mono, x=o + MONO_LEFT * t, baseline=o + MONO_BASELINE * t,
                       cap=MONO_CAP * t * (1.04 if small else 1.0), fill=C["text"], tracking=MONO_TRACK,
                       condense=MONO_CONDENSE)
    border_w = (0.040 if small else 0.0105) * t
    defs, glow_el = "", ""
    if glow and not small:
        defs = (f'<defs><filter id="g" x="-20%" y="-20%" width="140%" height="140%">'
                f'<feGaussianBlur stdDeviation="{0.020 * t:.1f}"/></filter>'
                f'<radialGradient id="v" cx="0.5" cy="0.45" r="0.7"><stop offset="0" stop-color="{C["bg"]}"/>'
                f'<stop offset="1" stop-color="#040506"/></radialGradient></defs>')
        glow_el = (f'<rect x="{o:.1f}" y="{o:.1f}" width="{t:.1f}" height="{t:.1f}" rx="{rx:.1f}" fill="none" '
                   f'stroke="{accent}" stroke-width="{border_w * 2.6:.1f}" opacity="0.55" filter="url(#g)"/>')
    fill = "url(#v)" if (glow and not small) else C["bg"]
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}" width="{size}" height="{size}">
  <!-- {APPS[app]["name"]} icon. Ubunye Creative Studio, Ubunye AI Ecosystems. MIT OR Apache-2.0.
       Letter outlines from DM Serif Display (SIL OFL 1.1). Generated by branding/tools/build.py. -->
  {defs}
  {glow_el}
  <rect x="{o:.1f}" y="{o:.1f}" width="{t:.1f}" height="{t:.1f}" rx="{rx:.1f}" fill="{fill}"/>
  {letters}
  <g>
    {lens(t, o, o, accent, small)}
  </g>
  <rect x="{o + border_w / 2:.1f}" y="{o + border_w / 2:.1f}" width="{t - border_w:.1f}" height="{t - border_w:.1f}" rx="{rx - border_w / 2:.1f}" fill="none" stroke="{accent}" stroke-width="{border_w:.1f}"/>
</svg>
'''


# ---------------------------------------------------------------- wordmark and lockups
def glyph_x(font: TTFont, text: str, index: int, tracking: float = 0.0) -> float:
    """Advance (font units) before character `index`."""
    gs, cm, upm = font.getGlyphSet(), font.getBestCmap(), font["head"].unitsPerEm
    return sum(gs[cm[ord(c)]].width + tracking * upm for c in text[:index])


def studio_wordmark(theme: str) -> str:
    ink = C["text"] if theme == "dark" else C["ink"]
    bg = C["bg"] if theme == "dark" else C["paper"]
    W, H, cap, track = 1200, 420, 190, -0.015
    word, ww = place(DISPLAY, "Ubunye", x=W / 2, baseline=250, cap=cap, fill=ink, tracking=track, anchor="middle")
    x0 = W / 2 - ww / 2
    s = cap / cap_units(DISPLAY)
    # As in the reference, CREATIVE STUDIO starts under the U's bowl and stops just before the
    # tail of the y, which curls up beside it.
    sub_left = x0 + 0.22 * cap
    sub_right = x0 + glyph_x(DISPLAY, "Ubunye", 4, track) * s - 0.06 * cap
    _, sw = place(LABEL, "CREATIVE STUDIO", x=0, baseline=0, cap=30, fill=ink, tracking=0.42)
    lab_cap = 30 * (sub_right - sub_left) / sw
    sub, _ = place(LABEL, "CREATIVE STUDIO", x=sub_left, baseline=318, cap=lab_cap, fill=ink, tracking=0.42)
    rule = f'<rect x="{W / 2 - 50}" y="368" width="100" height="4" fill="{C["light"] if theme == "dark" else C["light-on-light-bg"]}"/>'
    return "\n".join([
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">',
        f'  <!-- Ubunye Creative Studio wordmark ({theme}). DM Serif Display and Inter (SIL OFL 1.1) as outlines. -->',
        f'  <rect width="{W}" height="{H}" fill="{bg}"/>',
        f"  {word}", f"  {sub}", f"  {rule}", "</svg>", ""])


def app_lockup(app: str, layout: str, theme: str) -> tuple[str, int, int]:
    ink = C["text"] if theme == "dark" else C["ink"]
    bg = C["bg"] if theme == "dark" else C["paper"]
    acc = C[app] if theme == "dark" else C[f"{app}-on-light-bg"]
    second = APPS[app]["name"].split()[1]
    icon_inner = icon_svg(app, size=1024).split("\n", 1)[1].rsplit("</svg>", 1)[0]
    _, wa1 = place(DISPLAY, "Ubunye ", x=0, baseline=0, cap=100, fill=ink)
    _, wb1 = place(DISPLAY, second, x=0, baseline=0, cap=100, fill=ink)
    name_w100 = wa1 + wb1  # name width at cap height 100
    if layout == "horizontal":
        H, isz, pad, cap = 420, 340, 40, 120
        tx = pad + isz + 50
        W = int(tx + name_w100 * cap / 100 + pad + 20)
        ix, iy = pad, pad
        a, wa = place(DISPLAY, "Ubunye ", x=tx, baseline=258, cap=cap, fill=ink)
        b, _ = place(DISPLAY, second, x=tx + wa, baseline=258, cap=cap, fill=acc)
        lab, _ = place(LABEL, APPS[app]["category"], x=tx + 6, baseline=336, cap=24, fill=ink, tracking=0.42)
    else:
        W, H, isz = 900, 1000, 460
        cap = min(110, (W - 120) * 100 / name_w100)
        ix, iy = (W - isz) / 2, 60
        x0 = (W - name_w100 * cap / 100) / 2
        a, wa = place(DISPLAY, "Ubunye ", x=x0, baseline=700, cap=cap, fill=ink)
        b, _ = place(DISPLAY, second, x=x0 + wa, baseline=700, cap=cap, fill=acc)
        lab, _ = place(LABEL, APPS[app]["category"], x=W / 2, baseline=800, cap=24, fill=ink, tracking=0.42, anchor="middle")
    s = isz / 1024
    svg = "\n".join([
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">',
        f'  <rect width="{W}" height="{H}" fill="{bg}"/>',
        f'  <g transform="translate({ix} {iy}) scale({s:.5f})">{icon_inner}</g>',
        f"  {a}", f"  {b}", f"  {lab}", "</svg>", ""])
    return svg, W, H


# ---------------------------------------------------------------- tokens
def write_tokens() -> None:
    d = ROOT / "design-tokens"
    css = [":root {"] + [f"  --ubunye-{k}: {v};" for k, v in C.items()]
    css += [f'  --ubunye-font-display: "{TOK["font"]["display"]}", serif;',
            f'  --ubunye-font-label: "{TOK["font"]["label"]}", sans-serif;',
            f'  --ubunye-label-tracking: {TOK["font"]["label-tracking-em"]}em;', "}"]
    (d / "tokens.css").write_text("\n".join(css) + "\n", encoding="utf-8")
    ts = "// Generated from tokens.json by branding/tools/build.py. Do not edit.\n"
    ts += "export const ubunyeTokens = " + json.dumps({k: v for k, v in TOK.items() if not k.startswith("$")}, indent=2) + " as const;\n"
    (d / "tokens.ts").write_text(ts, encoding="utf-8")
    rs = ["// Generated from tokens.json by branding/tools/build.py. Do not edit.", ""]
    for k, v in C.items():
        r, g, b = (int(v[i:i + 2], 16) for i in (1, 3, 5))
        rs.append(f"pub const {k.upper().replace('-', '_')}: [u8; 3] = [{r}, {g}, {b}];")
    (d / "tokens.rs").write_text("\n".join(rs) + "\n", encoding="utf-8")


# ---------------------------------------------------------------- raster
def render(svg: pathlib.Path, png: pathlib.Path, w: int, h: int | None = None) -> None:
    png.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([RESVG, "-w", str(w), "-h", str(h or w), str(svg), str(png)], check=True)


def rasterize_icons(app: str) -> None:
    from PIL import Image
    d = ROOT / "icons" / app
    sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
    for s in sizes:
        src = d / (f"{app}-icon-small.svg" if s <= 32 else f"{app}-icon.svg")
        render(src, d / "png" / f"{app}-{s}.png", s)
    for s in [16, 24, 32, 48, 64, 128, 256, 512]:
        hd = d / "hicolor" / f"{s}x{s}" / "apps"
        hd.mkdir(parents=True, exist_ok=True)
        shutil.copy(d / "png" / f"{app}-{s}.png", hd / f"{APP_IDS[app]}.png")
    sc = d / "hicolor" / "scalable" / "apps"
    sc.mkdir(parents=True, exist_ok=True)
    shutil.copy(d / f"{app}-icon.svg", sc / f"{APP_IDS[app]}.svg")
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    imgs = [Image.open(d / "png" / f"{app}-{s}.png").convert("RGBA") for s in ico_sizes]
    imgs[-1].save(d / f"{app}.ico", sizes=[(s, s) for s in ico_sizes], append_images=imgs[:-1])
    for s in [16, 32, 64, 128, 256, 512, 1024]:
        render(d / f"{app}-icon-macos.svg" if s > 32 else d / f"{app}-icon-small.svg", d / "macos" / f"{s}.png", s)
    big = Image.open(d / "macos" / "1024.png").convert("RGBA")
    big.save(d / f"{app}.icns", append_images=[Image.open(d / "macos" / f"{s}.png").convert("RGBA") for s in [16, 32, 64, 128, 256, 512]])


def main() -> None:
    svg_only = "--svg-only" in sys.argv
    write_tokens()
    for app in APPS:
        d = ROOT / "icons" / app
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{app}-icon.svg").write_text(icon_svg(app), encoding="utf-8", newline="\n")
        (d / f"{app}-icon-small.svg").write_text(icon_svg(app, margin=0.0, small=True), encoding="utf-8", newline="\n")
        (d / f"{app}-icon-macos.svg").write_text(icon_svg(app, margin=0.098), encoding="utf-8", newline="\n")
        for layout in ("horizontal", "vertical"):
            for theme in ("dark", "light"):
                p = ROOT / "logos" / app / f"{app}-{layout}-{theme}.svg"
                p.parent.mkdir(parents=True, exist_ok=True)
                svg, w, h = app_lockup(app, layout, theme)
                p.write_text(svg, encoding="utf-8", newline="\n")
                if not svg_only:
                    render(p, p.with_suffix(".png"), w, h)
        if not svg_only:
            rasterize_icons(app)
    for theme in ("dark", "light"):
        p = ROOT / "wordmarks" / f"ubunye-creative-studio-{theme}.svg"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(studio_wordmark(theme), encoding="utf-8", newline="\n")
        if not svg_only:
            render(p, p.with_suffix(".png"), 1200, 420)
    print("built", ROOT)


if __name__ == "__main__":
    main()
