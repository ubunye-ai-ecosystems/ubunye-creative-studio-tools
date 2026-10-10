#!/usr/bin/env python3
"""Prepare a local ubunye-creative-* fork from its upstream, with full history.

    python3 -I scripts/prepare_fork.py <light|photo|film> <workdir> [--source <local upstream clone>]

Result in <workdir>/<slug>:
  * every upstream commit and tag (nothing squashed or rewritten)
  * remotes: origin = ubunye-ai-ecosystems/<slug>, upstream = storytold/<up> (push disabled)
  * branches: main and upstream-main = upstream HEAD; develop = upstream HEAD + one Ubunye scaffolding commit
Nothing is pushed. Publish with scripts/publish_fork.sh once the GitHub repository exists.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

STUDIO = pathlib.Path(__file__).resolve().parent.parent
OVERLAY = STUDIO / "fork-overlay"
IDS = tomllib.loads((STUDIO / "brand" / "identities.toml").read_text())

LINUX_BASE = "libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev"

APPS = {
    "light": dict(
        UP="lightcraft", UPNAME="LightCraft", SHORT="Light", KIND="RAW photo library and development",
        UPENV="LIGHTCRAFT", ENVPFX="UBUNYE_LIGHT", MSRV="1.90", LINUX_EXTRA="", WEB_TOOL=" =0.2.129",
        FORMATS="DNG, CR2, CR3, ARW, NEF, RAF, ORF, PEF, RW2, SRW, TIFF, JPEG, PNG, HEIF, XMP, Lightroom .lrcat, presets",
        FUZZ="proptest robustness suites only; no coverage-guided fuzzing yet (Milestone 2.5)",
        CONTROL="**The upstream control port has no authentication** (fix planned in Milestone 2.7).",
        ADVISORIES="- RUSTSEC-2023-0071 `rsa` 0.9.10, plus RUSTSEC-2026-0049/0098/0099/0104 `rustls-webpki` 0.102.8, both via `rustls-rustcrypto` 0.0.2-alpha (`crates/fetch`). Fix: switch to the rustls default provider.\n- RUSTSEC-2026-0194/0195 `quick-xml` 0.38.4 (XMP parsing). Fix: bump to 0.41.",
        DENY_IGNORES='  "RUSTSEC-2023-0071",  # rsa via rustls-rustcrypto (M2.3)\n  "RUSTSEC-2026-0049", "RUSTSEC-2026-0098", "RUSTSEC-2026-0099", "RUSTSEC-2026-0104",  # rustls-webpki 0.102 (M2.3)\n  "RUSTSEC-2026-0194", "RUSTSEC-2026-0195",  # quick-xml 0.38 (M2.3)\n  "RUSTSEC-2024-0436", "RUSTSEC-2026-0192",  # unmaintained: paste, ttf-parser (informational)',
        KNOWN="no 3-OS test CI upstream; unfuzzed parsers; unauthenticated control port; macOS Info.plist carries PhotoCraft UTIs; `crates/segment` and `crates/fetch` are Apache-2.0 only.",
        TOUCHPOINTS="- \"LightCraft\": about 1,201 occurrences, 526 of them in locale files\n- `APP_ID` in `apps/lightcraft/src/main.rs`; `packaging/macos/Info.plist.in`; `packaging/linux/ai.storyteller.lightcraft.*`; `packaging/windows/lightcraft.wxs` (UpgradeCode, Manufacturer, registry key, ARP URLs)\n- icons in `assets/app-icon/` (lynx); ArtCraft logos in `docs/brand/`\n- 32 `LIGHTCRAFT_*` env vars; config dir names; XMP namespace `ns.lightcraft.app` (**keep reading it** for compatibility)\n- AppImage update info in `packaging/linux/package.sh`",
        BUILD_ROW="| 2026-10-10 | Linux x86_64, 4 vCPU, rustc 1.97.0, no GPU | `cargo build --locked -p lightcraft-cli` | ✅ 13 min 41 s; smoke test passed (CPU path) |",
    ),
    "photo": dict(
        UP="photocraft", UPNAME="PhotoCraft", SHORT="Photo", KIND="Layered image editing",
        UPENV="PHOTOCRAFT", ENVPFX="UBUNYE_PHOTO", MSRV="1.95", LINUX_EXTRA=" libgtk-3-dev", WEB_TOOL=" + trunk 0.21.14",
        FORMATS="PSD/PSB, .pcraft, PDN, Affinity, ORA, TIFF, PNG, JPEG, WebP, HEIF, camera raw, ICC, ABR, SVG, fonts",
        FUZZ="16 cargo-fuzz targets upstream, run manually only; nightly runs planned in Milestone 2.5",
        CONTROL="Upstream's control channel is token-protected and bound to loopback.",
        ADVISORIES="- none (0 vulnerabilities). Warnings: unmaintained `paste`, `ttf-parser`, `rustybuzz`; yanked `yoke-derive` 0.8.3.",
        DENY_IGNORES='  "RUSTSEC-2024-0436", "RUSTSEC-2026-0192", "RUSTSEC-2026-0206",  # unmaintained: paste, ttf-parser, rustybuzz (informational)',
        KNOWN="GPU compositor panic in the 150-layer scenario; 46 of 149 settings do nothing; no macOS PR CI; NOTICE/ATTRIBUTION missing from most packages; 8 locales unattributed; the CC BY 3.0 lasso icon needs visible credit.",
        TOUCHPOINTS="- \"PhotoCraft\": more than 12,000 occurrences, including 16 locale files\n- `apps/photocraft/src/main.rs:54` app id; `packaging/macos/Info.plist.in` (UTTypes `.document`, `.ora`, `.pdn`, `.qoi`); WiX UpgradeCode, Manufacturer, ProgIDs `Photocraft.*`; MIME `application/x-photocraft`\n- icons in `assets/app-icon/` (kitsune); ArtCraft logos in `docs/brand/`\n- 56 `PHOTOCRAFT_*` env vars; nfpm vendor and homepage; AppImage update info (`packaging/linux/package.sh:128`)",
        BUILD_ROW="{{PHOTO_BUILD}}",
    ),
    "film": dict(
        UP="filmcraft", UPNAME="FilmCraft", SHORT="Film", KIND="Video editing, colour and sound",
        UPENV="FILMCRAFT", ENVPFX="UBUNYE_FILM", MSRV="1.95", LINUX_EXTRA=" libasound2-dev", WEB_TOOL=" =0.2.129",
        FORMATS="MP4/MOV, MKV/WebM, MXF, MPEG-TS, Ogg, WAV, AAF, OMF, FCP XML, H.264, HEVC, VP9, AV1, ProRes, DNx, MPEG-2, AAC, Opus, AC-3, captions, LUTs",
        FUZZ="deterministic mutation tests only; no cargo-fuzz harnesses yet (Milestone 2.5)",
        CONTROL="**The upstream control port has no authentication** and can write files anywhere while enabled (fix planned in Milestone 2.7).",
        ADVISORIES="- RUSTSEC-2023-0071 `rsa` 0.9.10, plus RUSTSEC-2026-0049/0098/0099/0104 `rustls-webpki` 0.102.8, both via `rustls-rustcrypto` 0.0.2-alpha (`crates/speech`, optional `download` feature).\n- Warnings: unsound `glib` 0.18 (Linux menus via muda); unmaintained `paste`, `proc-macro-error`.",
        DENY_IGNORES='  "RUSTSEC-2023-0071",  # rsa via rustls-rustcrypto (M2.3)\n  "RUSTSEC-2026-0049", "RUSTSEC-2026-0098", "RUSTSEC-2026-0099", "RUSTSEC-2026-0104",  # rustls-webpki 0.102 (M2.3)\n  "RUSTSEC-2024-0429",  # glib 0.18 unsound (via muda/gtk)\n  "RUSTSEC-2024-0436", "RUSTSEC-2024-0370",  # unmaintained: paste, proc-macro-error',
        KNOWN="no CI runs tests upstream; codec patent review pending (H.264/HEVC/AAC); Adobe feature names in the UI (Lumetri, Essential Sound, Ultra Key, Warp Stabilizer); the Linux MIME XML appears empty; Kokoro and CMUdict unattributed.",
        TOUCHPOINTS="- \"FilmCraft\": hundreds of occurrences; ArtCraft logos compiled in by `crates/ui-egui/src/brand.rs` (`include_bytes!`), **must be removed**\n- app id `ai.storyteller.filmcraft` (Info.plist, UTType `.fcproj`, Flatpak, desktop, metainfo); `packaging/windows/filmcraft.wxs` UpgradeCode and Manufacturer\n- icons in `assets/app-icon/` (owl); `FILMCRAFT_*` env vars; AppImage update channel `storytold`\n- Adobe feature labels: \"Lumetri\" (136 in `.rs`), \"Premiere Pro Compatible\" shortcut preset (`crates/engine/src/shortcuts.rs:60`)",
        BUILD_ROW="{{FILM_BUILD}}",
    ),
}

PTR = {
    "AGENTS.md": "> **Ubunye fork:** read [`ubunye/CLAUDE.md`](ubunye/CLAUDE.md) first; its rules take precedence.\n\n",
    "CLAUDE.md": "> **Ubunye fork:** read [`ubunye/CLAUDE.md`](ubunye/CLAUDE.md) first; its rules take precedence.\n\n",
    "SECURITY.md": "> **Ubunye fork:** report vulnerabilities in this distribution as described in [`ubunye/SECURITY.md`](ubunye/SECURITY.md).\n\n",
}


def sh(*cmd: str, cwd: pathlib.Path | None = None) -> str:
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def render(text: str, v: dict[str, str]) -> str:
    # Only {{UPPER_CASE}} placeholders; GitHub expressions use ${{ lower.case }} and are left alone.
    return re.sub(r"(?<!\$)\{\{([A-Z_]+)\}\}", lambda m: v.get(m.group(1), m.group(0)), text)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("app", choices=APPS)
    ap.add_argument("workdir")
    ap.add_argument("--source", help="local clone of upstream to copy objects from (still verified against upstream URL)")
    ap.add_argument("--build-row", default=None, help="markdown table row for the verified build log")
    a = ap.parse_args()

    d = dict(APPS[a.app])
    ident = IDS[a.app]
    d.update(
        NAME=ident["name"], APPID=ident["app_id"], UPGRADE=ident["windows_upgrade_code"],
        SLUG=f"ubunye-creative-{a.app}", SHORT_LC=a.app, CLI=f"{d['UP']}-cli",
        UPURL=f"https://github.com/storytold/{d['UP']}",
    )
    if a.build_row:
        d["BUILD_ROW"] = a.build_row
    elif d["BUILD_ROW"].startswith("{{"):
        d["BUILD_ROW"] = "| — | — | — | not yet verified |"

    work = pathlib.Path(a.workdir).resolve()
    repo = work / d["SLUG"]
    if repo.exists():
        print(f"{repo} exists; refusing to overwrite", file=sys.stderr)
        return 1
    work.mkdir(parents=True, exist_ok=True)

    src = a.source or (d["UPURL"] + ".git")
    sh("git", "clone", "--no-local", src, str(repo))
    sh("git", "remote", "rename", "origin", "upstream", cwd=repo)
    sh("git", "remote", "set-url", "upstream", d["UPURL"] + ".git", cwd=repo)
    sh("git", "remote", "set-url", "--push", "upstream", "DISABLED", cwd=repo)
    sh("git", "remote", "add", "origin", f"https://github.com/ubunye-ai-ecosystems/{d['SLUG']}.git", cwd=repo)
    sh("git", "fetch", "--tags", "upstream", cwd=repo)
    head = sh("git", "rev-parse", "upstream/main", cwd=repo)
    sh("git", "checkout", "-q", "-B", "main", head, cwd=repo)
    sh("git", "branch", "-f", "upstream-main", head, cwd=repo)
    sh("git", "checkout", "-q", "-b", "develop", cwd=repo)
    # Keep only main + tags locally (upstream's work branches are not imported).
    for ref in sh("git", "for-each-ref", "--format=%(refname:short)", "refs/heads", cwd=repo).splitlines():
        if ref not in ("main", "upstream-main", "develop"):
            sh("git", "branch", "-D", ref, cwd=repo)

    count = int(sh("git", "rev-list", "--count", head, cwd=repo))
    date = sh("git", "log", "-1", "--format=%cI", head, cwd=repo)
    version = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    tag = sh("git", "describe", "--tags", "--abbrev=0", head, cwd=repo) if count else None
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    for f in OVERLAY.rglob("*"):
        if f.is_dir():
            continue
        rel = f.relative_to(OVERLAY)
        out = repo / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(render(f.read_text(), d))
    (repo / "ubunye" / "scripts").mkdir(parents=True, exist_ok=True)
    shutil.copy(STUDIO / "scripts" / "upstream_report.py", repo / "ubunye" / "scripts" / "upstream_report.py")
    smoke = STUDIO / "templates" / f"smoke-{a.app}.sh"
    if smoke.exists():
        shutil.copy(smoke, repo / "ubunye" / "scripts" / "smoke.sh")
        (repo / "ubunye" / "scripts" / "smoke.sh").chmod(0o755)

    state = {
        "$schema_version": 1,
        "app": d["SLUG"],
        "upstream": {"name": d["UPNAME"], "url": d["UPURL"] + ".git", "branch": "main", "license": "MIT OR Apache-2.0"},
        "last_checked_at": now,
        "last_integrated": {
            "sha": head, "date": date, "upstream_version": version, "upstream_tag": tag,
            "method": "initial-import", "merged_pr": None, "merged_by": None, "merged_at": None,
        },
        "pending": None,
        "history": [{"sha": head, "outcome": "initial-import", "commits": count, "conflicts": 0, "ci": "not-run", "recorded_at": now}],
    }
    (repo / "UPSTREAM_STATE.json").write_text(json.dumps(state, indent=2) + "\n")

    patched = []
    for name, line in PTR.items():
        p = repo / name
        if p.exists():
            p.write_text(line + p.read_text())
            patched.append(name)
    (repo / "ubunye" / "patched-paths.txt").write_text(
        "# Upstream files modified by Ubunye (one path per line). Used for conflict-hotspot reports.\n"
        + "".join(f"{x}\n" for x in patched)
    )

    sh("git", "add", "-A", cwd=repo)
    sh("git", "commit", "-q", "-m",
       f"Add Ubunye fork scaffolding for {d['NAME']}\n\n"
       f"Imports {d['UPNAME']} at {head[:12]} ({count} commits, upstream v{version}) with full history.\n"
       "Adds upstream tracking (UPSTREAM.md, UPSTREAM_STATE.json), a pinned toolchain, a cargo-deny\n"
       "policy, Ubunye CI/audit/upstream-sync workflows and Ubunye docs under ubunye/. Upstream\n"
       "LICENSE, NOTICE and ATTRIBUTION files are unchanged; only one-line pointers were prepended to\n"
       + ", ".join(patched) + ".", cwd=repo)
    print(json.dumps({"repo": str(repo), "upstream_sha": head, "commits": count, "version": version, "tag": tag, "patched": patched}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
