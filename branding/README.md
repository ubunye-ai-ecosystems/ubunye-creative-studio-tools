# Ubunye Creative Studio brand

The brand for Ubunye Creative Studio and its three apps: Ubunye Light, Ubunye Photo and Ubunye Film.
The approved reference board (2026-10-11) is the source of truth. Everything here is generated from it by code, so it can be rebuilt and checked.

## What is here

| Folder | Contents |
|---|---|
| `design-tokens/` | `tokens.json` (edit this one), and generated `tokens.css`, `tokens.ts`, `tokens.rs` |
| `icons/<app>/` | `<app>-icon.svg` (master, 1024), `-small.svg` (for 16 to 32 px), `-macos.svg` (Apple grid), `png/` 16 to 1024, `<app>.ico`, `<app>.icns`, `hicolor/` for Linux |
| `logos/<app>/` | horizontal and vertical lockups, dark and light, SVG and PNG |
| `wordmarks/` | the "Ubunye / CREATIVE STUDIO" wordmark, dark and light |
| `fonts/` | DM Serif Display and Inter, both SIL OFL 1.1, with their licences |
| `marketing/` | brand board and website card photos |
| `documentation/qa/` | renders compared with the reference board |
| `tools/build.py` | builds everything above |

## Rebuild

```
pip install fonttools pillow
cargo install resvg          # or set RESVG to a resvg binary
python branding/tools/build.py
```

Change a colour or name in `design-tokens/tokens.json`, then rebuild. Icon geometry (lens, rays, letters) is set by the constants at the top of the icon section in `build.py`, measured as fractions of the tile from the reference board.

## Rules

- The SVGs are real vector drawings. Letters are converted to outlines, so no font needs to be installed, and no raster image is embedded.
- Below 48 px use the `-small` icon: fewer, thicker rays, a plain lens and no glow, so it stays readable.
- The monogram is DM Serif Display, tightened and condensed to 86 percent width to match the narrow letters in the reference.
- On light backgrounds the accent text uses the darker `*-on-light-bg` colours so it stays readable; the icons themselves always keep their dark tile.
- No trademark symbol is used.

## Licences

Brand artwork: Ubunye AI Ecosystems. The apps are based on LightCraft, PhotoCraft and FilmCraft by the ArtCraft team (MIT OR Apache-2.0); none of the ArtCraft marks are used here. Fonts: SIL Open Font License 1.1 (`fonts/OFL-*.txt`).
