"""E37c: build test package v8 for the real machines from one `peer setup` team.

Windows zip: home laptop (Member0) and Soshanguve laptop (Bob). Mac zip: Member1, its own helper
key (v7's Mac package reused Soshanguve's key: two helpers with one id answer each other's
requests). Every package carries every member's certificate, because a serving device lets a
teammate fetch media only if it finds that teammate's certificate next to its own folder.
Keys and device folders are copied, never printed.

Usage: python e37c_package.py <team dir> <keys dir> <windows bin dir> <mac bin dir> <out dir>
  keys dir holds home.key, soshanguve.key, mac.key; out dir gets kopano-test-v8.zip and
  kopano-test-v8-mac.zip"""
import json, os, shutil, sys, zipfile

TEAM, KEYS, WIN, MAC, OUT = sys.argv[1:6]
# E39: v9 adds `netcheck` and download progress; the Mac zip is skipped when MAC is 'none'
V = sys.argv[6] if len(sys.argv) > 6 else 'v9'
OWNER = json.load(open(os.path.join(TEAM, 'alice', 'cert.json')))['body']['device']
MEMBERS = ['alice', 'bob', 'm0', 'm1']
SKIP = {'addr.json', 'serve-errors.log', 'media'}  # originals come over the network in the test

WIN_START = r'''@echo off
cd /d "%~dp0"
title Kopano test VER - {title}
if not exist "%~dp0agent.exe" (
  echo This is running from inside the zip file.
  echo Please right-click the zip, choose Extract All, then run this file from the extracted folder.
  pause
  exit /b 1
)
"%~dp0agent.exe" --dir "%~dp0team\{member}" --key "%~dp0keys\{key}" --owner {owner}
pause
'''

MAC_START = '''#!/bin/bash
# Kopano test helper VER for a Mac. Double-click (first time: right-click > Open).
cd "$(dirname "$0")" || exit 1
xattr -dr com.apple.quarantine . 2>/dev/null
chmod +x peer agent
./agent --dir "$PWD/team/m1" --key "$PWD/keys/mac.key" --owner {owner}
echo
read -r -p "Press Enter to close."
'''

MAC_README = '''Kopano test on a Mac (ThabangVision), version VER, about 30 minutes

1. Double-click the zip to unpack it (the Downloads folder is fine). Delete any old test folder.
2. Open the folder. RIGHT-click START-Mac.command and choose Open.
   macOS says it is from an unidentified developer: click Open.
   (If there is no Open button: System Settings > Privacy & Security, scroll down,
    click "Open Anyway" next to START-Mac.command, then try again.)
3. A Terminal window opens. If macOS asks to allow incoming network connections, click Allow.
4. Keep the Mac plugged in and awake. Leave the window open. When it says "online", you are done.
5. When Thabang says the test is finished, close the window and delete the folder.

What it does: it keeps a small test project online (public sample photos and test files, no
personal files) and only follows instructions from Thabang's PC. It installs nothing, and it
only reads (never changes) the network settings when checking why a connection fails.
'''


def team_into(root, full):
    for m in MEMBERS:
        src, dst = os.path.join(TEAM, m), os.path.join(root, 'team', m)
        if m in full:
            shutil.copytree(src, dst, ignore=lambda d, names: [n for n in names if n in SKIP])
        else:
            os.makedirs(dst)
            shutil.copy2(os.path.join(src, 'cert.json'), dst)


def zip_dir(root, out, exec_names=()):
    with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as z:
        for d, _, files in os.walk(root):
            for f in files:
                p = os.path.join(d, f)
                arc = os.path.join(os.path.basename(root), os.path.relpath(p, root))
                info = zipfile.ZipInfo.from_file(p, arc)
                if f in exec_names:
                    info.external_attr = 0o100755 << 16  # Finder keeps the run permission
                with open(p, 'rb') as fh:
                    z.writestr(info, fh.read(), zipfile.ZIP_DEFLATED)


os.makedirs(OUT, exist_ok=True)
here = os.path.dirname(os.path.abspath(__file__))

# Windows
w = os.path.join(OUT, f'kopano-test-{V}')
shutil.rmtree(w, ignore_errors=True)
os.makedirs(os.path.join(w, 'keys'))
for f in ('agent.exe', 'peer.exe'):
    shutil.copy2(os.path.join(WIN, f), w)
for f in ('home.key', 'soshanguve.key'):
    shutil.copy2(os.path.join(KEYS, f), os.path.join(w, 'keys'))
open(os.path.join(w, 'README.txt'), 'w', newline='\r\n').write(open(os.path.join(here, 'e37c_package_files', 'README.txt')).read().replace('version 8', 'version ' + V.lstrip('v')))
for f in ('ALLOW-same-wifi.cmd', 'REMOVE-same-wifi.cmd'):
    shutil.copy2(os.path.join(here, 'e37c_package_files', f), w)
for name, member, key in (('home laptop', 'm0', 'home.key'), ('Soshanguve laptop', 'bob', 'soshanguve.key')):
    fn = 'START-home-laptop.cmd' if member == 'm0' else 'START-Soshanguve-laptop.cmd'
    open(os.path.join(w, fn), 'w', newline='\r\n').write(WIN_START.replace('VER', V).format(title=name, member=member, key=key, owner=OWNER))
team_into(w, {'bob', 'm0'})
zip_dir(w, os.path.join(OUT, f'kopano-test-{V}.zip'))

# Mac
if MAC != 'none':
    m = os.path.join(OUT, f'kopano-test-{V}-mac')
    shutil.rmtree(m, ignore_errors=True)
    os.makedirs(os.path.join(m, 'keys'))
    for f in ('agent', 'peer'):
        shutil.copy2(os.path.join(MAC, f), m)
    shutil.copy2(os.path.join(KEYS, 'mac.key'), os.path.join(m, 'keys'))
    open(os.path.join(m, 'START-Mac.command'), 'w', newline='\n').write(MAC_START.replace('VER', V).format(owner=OWNER))
    open(os.path.join(m, 'README-Mac.txt'), 'w', newline='\n').write(MAC_README.replace('VER', V))
    team_into(m, {'m1'})
    zip_dir(m, os.path.join(OUT, f'kopano-test-{V}-mac.zip'), exec_names=('agent', 'peer', 'START-Mac.command'))

for z in (f'kopano-test-{V}.zip',) + ((f'kopano-test-{V}-mac.zip',) if MAC != 'none' else ()):
    p = os.path.join(OUT, z)
    print(json.dumps({'zip': z, 'mb': round(os.path.getsize(p) / 1e6, 1), 'files': len(zipfile.ZipFile(p).namelist())}))
