"""Publish an app's draft GitHub release to downloads.thabangvision.com without passing the files
through this machine: big files are copied GitHub -> R2 by the ubunye-creative-ingest Worker.

    python publish_from_github.py <light|photo|film> [--dry-run]

Env: GITHUB_TOKEN (reads the draft release), INGEST_KEY (the Worker's secret).
Small .zsync files are fetched here, renamed inside (Filename/URL) and uploaded with wrangler.
Layout and manifest: see publish_release.py.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from publish_release import BASE, BUCKET, TYPES, UPSTREAM, rewrite_zsync

ORG = "ubunye-ai-ecosystems"
INGEST = "https://ingest.tools.thabangvision.com/"
GH = os.environ["GITHUB_TOKEN"]
KEY = os.environ["INGEST_KEY"]

# Optional: pin the ingest host's IP when a local resolver still caches "no such name"
# after the custom domain was created (INGEST_IP=<a Cloudflare edge IPv4>).
if os.environ.get("INGEST_IP"):
    import socket

    _orig_getaddrinfo = socket.getaddrinfo

    def _getaddrinfo(host, *args, **kwargs):
        if host == "ingest.tools.thabangvision.com":
            host = os.environ["INGEST_IP"]
        return _orig_getaddrinfo(host, *args, **kwargs)

    socket.getaddrinfo = _getaddrinfo


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


def gh(path: str):
    req = urllib.request.Request("https://api.github.com" + path,
                                 headers={"Authorization": "Bearer " + GH, "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def signed_url(repo: str, asset_id: int) -> str:
    req = urllib.request.Request(f"https://api.github.com/repos/{ORG}/{repo}/releases/assets/{asset_id}",
                                 headers={"Authorization": "Bearer " + GH, "Accept": "application/octet-stream"})
    try:
        urllib.request.build_opener(NoRedirect).open(req)
    except urllib.error.HTTPError as e:
        if e.code in (301, 302, 307):
            return e.headers["Location"]
        raise
    raise RuntimeError("expected a redirect to the asset's storage URL")


def ctype(name: str) -> str:
    return next((t for ext, t in TYPES.items() if name.endswith(ext)), "application/octet-stream")


def ingest(url: str, key: str, size: int, name: str, dry: bool) -> None:
    print(f"  copy {key} ({size // 1048576} MB)", flush=True)
    if dry:
        return
    body = json.dumps({"url": url, "key": key, "size": size, "contentType": ctype(name)}).encode()
    for attempt in range(3):
        req = urllib.request.Request(INGEST, data=body, method="POST",
                                     headers={"x-ingest-key": KEY, "Content-Type": "application/json",
                                              "User-Agent": "ubunye-publish/1"})
        try:
            with urllib.request.urlopen(req, timeout=900) as r:
                out = json.loads(r.read())
            if out.get("size") == size:
                return
            raise RuntimeError(f"stored size {out.get('size')} != {size}")
        except Exception as e:  # noqa: BLE001
            print(f"    attempt {attempt + 1} failed: {str(e)[:200]}", flush=True)
    sys.exit(f"could not copy {key}")


def put_small(key: str, path: Path, dry: bool) -> None:
    print(f"  put {key}", flush=True)
    if dry:
        return
    subprocess.run(["npx.cmd" if sys.platform == "win32" else "npx", "-y", "wrangler@4", "r2", "object", "put",
                    f"{BUCKET}/{key}", "--file", str(path), "--content-type", ctype(path.name), "--remote"],
                   check=True, stdout=subprocess.DEVNULL)


def main() -> int:
    app = sys.argv[1]
    dry = "--dry-run" in sys.argv
    up, repo = UPSTREAM[app], f"ubunye-creative-{app}"
    drafts = [r for r in gh(f"/repos/{ORG}/{repo}/releases?per_page=10") if r["draft"]]
    if len(drafts) != 1:
        sys.exit(f"expected one draft release in {repo}, found {len(drafts)}")
    pat = re.compile(rf"^{up}(-cli)?-(\d+\.\d+\.\d+(?:-(?:rc|alpha|beta|pre)[0-9A-Za-z.]*)?)-(.+)$")
    assets = [a for a in drafts[0]["assets"] if pat.match(a["name"]) and "-web-" not in a["name"]
              and not a["name"].startswith(f"{up}-web")]
    versions = {pat.match(a["name"]).group(2) for a in assets}
    if len(versions) != 1:
        sys.exit(f"expected one version, found {versions}")
    version = versions.pop()
    manifest = {"app": app, "version": version,
                "published": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "files": []}
    with tempfile.TemporaryDirectory() as tmp:
        for a in sorted(assets, key=lambda a: a["name"]):
            m = pat.match(a["name"])
            name = f"ubunye-creative-{app}{m.group(1) or ''}-{version}-{m.group(3)}"
            key = f"{app}/{version}/{name}"
            am = re.match(r"^ubunye-creative-\w+-[^-]+-linux-(\w+)\.AppImage(\.zsync)?$", name)
            stable = f"ubunye-creative-{app}-linux-{am.group(1)}.AppImage{am.group(2) or ''}" if am else None
            if name.endswith(".zsync"):
                raw = Path(tmp) / a["name"]
                raw.write_bytes(urllib.request.urlopen(signed_url(repo, a["id"])).read())
                p = Path(tmp) / name
                rewrite_zsync(raw, p, name[: -len(".zsync")])
                put_small(key, p, dry)
                sp = Path(tmp) / stable
                rewrite_zsync(raw, sp, stable[: -len(".zsync")])
                put_small(f"{app}/stable/{stable}", sp, dry)
                size, digest = p.stat().st_size, hashlib.sha256(p.read_bytes()).hexdigest()
            else:
                url = signed_url(repo, a["id"])
                ingest(url, key, a["size"], name, dry)
                if stable:
                    ingest(signed_url(repo, a["id"]), f"{app}/stable/{stable}", a["size"], name, dry)
                size, digest = a["size"], (a.get("digest") or "").removeprefix("sha256:")
            manifest["files"].append({"name": name, "url": f"{BASE}/{key}", "size": size, "sha256": digest})
        mp = Path(tmp) / "latest.json"
        mp.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        put_small(f"{app}/latest.json", mp, dry)
    print(f"published {app} {version}: {len(manifest['files'])} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
