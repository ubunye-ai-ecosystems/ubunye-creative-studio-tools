"""Summarise an e15_drive.sh run: one line per step with result, path and timings."""
import json, re, sys

for line in open(sys.argv[1], encoding='utf-8', errors='replace'):
    line = line.strip()
    if not line.startswith('{'):
        if line and line != 'DRIVER_DONE':
            print('   ', line[:160])
        continue
    d = json.loads(line)
    out = d.get('out')
    if not isinstance(out, dict):
        print(f"{d['step'][:38]:38} {d['laptop']:10} no output")
        continue
    if 'reply' in out:  # went through a laptop's helper
        rep = out['reply']
        res = rep.get('result', {}) if rep.get('ok') else {}
        so = res.get('stdout', '') if isinstance(res, dict) else ''
        ev = [json.loads(x) for x in so.splitlines() if x.startswith('{')]
        e = ev[-1] if ev else {}
        status = 'ok' if rep.get('ok') and res.get('code', 0) == 0 else f"FAILED {rep.get('error') or res.get('stderr', '')[-160:]}"
        ctl = f"control {out.get('control_connect_ms')} ms via {re.sub(r'https://|/|\\.$', '', out.get('control_path', ''))[:30]}"
    else:  # Alice ran it here
        e, status, ctl = out, ('ok' if 'event' in out or 'result' in out else 'FAILED'), ''
    bits = []
    for k in ('path', 'connect_ms', 'transfer_ms', 'objects', 'bytes', 'media_bytes', 'result', 'save_ms', 'version', 'mbit_s', 'secs'):
        if k in e and e[k] not in ('', None):
            bits.append(f"{k}={e[k]}")
    if 'os' in (res if 'reply' in out and isinstance(res, dict) else {}):
        bits.append(f"os={res['os']} cpus={res['cpus']}")
    print(f"{d['step'][:38]:38} {d['laptop']:10} {status:6} {ctl:44} {' '.join(bits)[:220]}")
