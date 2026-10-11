"""Publish one app's release files to downloads.thabangvision.com (R2 bucket ubunye-creative-tools).

    python publish_release.py <light|photo|film> <dir with the release workflow's files> [--dry-run]

Layout in the bucket:
    <app>/<version>/ubunye-creative-<app>-<version>-<platform>.<ext>   every file, renamed from upstream's
    <app>/stable/ubunye-creative-<app>-linux-<arch>.AppImage(.zsync)  fixed names the AppImage updater reads
    <app>/latest.json                                                  manifest: version, files, sizes, sha256

Uploads with `npx wrangler r2 object put --remote` (wrangler must be logged in to the account).
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

BUCKET = "ubunye-creative-tools"
BASE = "https://downloads.thabangvision.com"
UPSTREAM = {"light": "lightcraft", "photo": "photocraft", "film": "filmcraft"}
TYPES = {
    ".msi": "application/x-msi", ".zip": "application/zip", ".dmg": "application/x-apple-diskimage",
    ".AppImage": "application/octet-stream", ".zsync": "application/octet-stream",
    ".deb": "application/vnd.debian.binary-package", ".rpm": "application/x-rpm",
    ".gz": "application/gzip", ".flatpak": "application/vnd.flatpak", ".json": "application/json",
}


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def put(key: str, path: Path, dry: bool) -> None:
    ctype = next((t for ext, t in TYPES.items() if path.name.endswith(ext)), "application/octet-stream")
    print(f"  put {key} ({path.stat().st_size // 1048576} MB)", flush=True)
    if dry:
        return
    subprocess.run(["npx.cmd" if sys.platform == "win32" else "npx", "-y", "wrangler@4", "r2", "object", "put",
                    f"{BUCKET}/{key}", "--file", str(path), "--content-type", ctype, "--remote"],
                   check=True, stdout=subprocess.DEVNULL)


def rewrite_zsync(src: Path, dst: Path, appimage_name: str) -> None:
    """Point the .zsync header (Filename, URL) at the renamed AppImage; the block data is unchanged."""
    data = src.read_bytes()
    head, sep, body = data.partition(b"\n\n")
    lines = []
    for line in head.split(b"\n"):
        if line.startswith(b"Filename: ") or line.startswith(b"URL: "):
            line = line.split(b": ", 1)[0] + b": " + appimage_name.encode()
        lines.append(line)
    dst.write_bytes(b"\n".join(lines) + sep + body)


def main() -> int:
    app, src = sys.argv[1], Path(sys.argv[2])
    dry = "--dry-run" in sys.argv
    up = UPSTREAM[app]
    pat = re.compile(rf"^{up}(-cli)?-(\d+\.\d+\.\d+(?:-(?:rc|alpha|beta|pre)[0-9A-Za-z.]*)?)-(.+)$")
    files = sorted(p for p in src.iterdir() if p.is_file() and pat.match(p.name))
    versions = {pat.match(p.name).group(2) for p in files}
    if len(versions) != 1:
        sys.exit(f"expected one version in {src}, found {versions}")
    version = versions.pop()
    manifest = {"app": app, "version": version, "published": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "files": []}
    with tempfile.TemporaryDirectory() as tmp:
        for p in files:
            m = pat.match(p.name)
            name = f"ubunye-creative-{app}{m.group(1) or ''}-{version}-{m.group(3)}"
            path = p
            if name.endswith(".AppImage.zsync"):
                path = Path(tmp) / name
                rewrite_zsync(p, path, name[: -len(".zsync")])
            key = f"{app}/{version}/{name}"
            put(key, path, dry)
            manifest["files"].append({"name": name, "url": f"{BASE}/{key}", "size": path.stat().st_size,
                                      "sha256": sha256(path)})
            # Fixed names for the AppImage updater (see packaging/linux/package.sh UPDATE_INFO).
            am = re.match(r"^ubunye-creative-\w+-[^-]+-linux-(\w+)\.AppImage(\.zsync)?$", name)
            if am:
                stable = f"ubunye-creative-{app}-linux-{am.group(1)}.AppImage{am.group(2) or ''}"
                spath = path
                if am.group(2):
                    spath = Path(tmp) / stable
                    rewrite_zsync(p, spath, stable[: -len(".zsync")])
                put(f"{app}/stable/{stable}", spath, dry)
        mpath = Path(tmp) / "latest.json"
        mpath.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        put(f"{app}/latest.json", mpath, dry)
        if dry:
            print(json.dumps(manifest, indent=2))
    print(f"published {app} {version}: {len(manifest['files'])} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
