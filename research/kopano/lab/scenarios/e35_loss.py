"""E35: a direct connection (no relay, no internet: LAN-only mode) through a UDP proxy that drops
a share of packets in both directions and adds delay. QUIC itself must cope with the loss.
Usage: python e35_loss.py <peer.exe> <work dir> <media MB>"""
import asyncio, json, os, random, shutil, subprocess, sys, threading, time

P, W, MB = sys.argv[1], sys.argv[2], int(sys.argv[3])
random.seed(35)
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '20000'], capture_output=True, check=True)
with open(f'{W}/clip.bin', 'wb') as f:
    for _ in range(MB):
        f.write(os.urandom(1 << 20))
h = json.loads(subprocess.run([P, 'add-media', '--dir', f'{W}/alice', '--file', f'{W}/clip.bin'], capture_output=True, text=True).stdout.strip().splitlines()[-1])['hash']
LOSSES = [float(x) for x in os.environ.get('E35_LOSSES', '0,0.01,0.05,0.10,0.20').split(',')]
ENV = dict(os.environ, KOPANO_LAN_ONLY='1')
CFG = {'loss': 0.0, 'delay': 0.05, 'bytes': 0}

class Proxy(asyncio.DatagramProtocol):
    """One client <-> one server, drops CFG['loss'] of datagrams each way, delays the rest."""
    def __init__(self, target):
        self.target, self.client, self.up = target, None, None
    def connection_made(self, t):
        self.t = t
    def datagram_received(self, data, addr):
        CFG['bytes'] += len(data)
        if random.random() < CFG['loss']:
            return
        loop = asyncio.get_running_loop()
        if addr == self.target:
            if self.client:
                loop.call_later(CFG['delay'], self.t.sendto, data, self.client)
        else:
            self.client = addr
            loop.call_later(CFG['delay'], self.t.sendto, data, self.target)

def run_proxy(target_port):
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    loop.run_until_complete(loop.create_datagram_endpoint(lambda: Proxy(('127.0.0.1', target_port)), local_addr=('127.0.0.1', 5000)))
    loop.run_forever()

addr_file = f'{W}/alice/addr.json'
alice = subprocess.Popen([P, 'serve', '--dir', f'{W}/alice'], env=ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(addr_file):
    time.sleep(0.3)
addr = json.load(open(addr_file))
port = int([a['Ip'] for a in addr['addrs'] if 'Ip' in a][0].rsplit(':', 1)[1])
json.dump({'id': addr['id'], 'addrs': [{'Ip': '127.0.0.1:5000'}]}, open(f'{W}/via-proxy.json', 'w'))
threading.Thread(target=run_proxy, args=(port,), daemon=True).start()
time.sleep(0.5)

def go(args):
    t = time.time()
    try:
        r = subprocess.run([P, *args], capture_output=True, text=True, env=ENV, timeout=900)
    except subprocess.TimeoutExpired:
        return None, 'timeout > 900 s', 900
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    return (ev[-1] if ev else None), r.stderr.strip()[-160:], round(time.time() - t, 1)

rows = []
for loss in LOSSES:
    CFG['loss'] = loss
    d = f'{W}/bob-{int(loss * 100)}'
    shutil.copytree(f'{W}/bob', d)
    e1, err1, s1 = go(['pull', '--dir', d, '--from', f'{W}/via-proxy.json', '--compact'])
    b0 = CFG['bytes']
    e2, err2, s2 = go(['fetch-media', '--dir', d, '--from', f'{W}/via-proxy.json', '--hash', h])
    row = {'loss_pct': int(loss * 100), 'rtt_ms': int(CFG['delay'] * 2000),
           'join_20k_items': {'s': s1, 'ok': bool(e1) and 'applied' in e1.get('result', ''), 'path': e1 and e1.get('path'), 'err': '' if e1 else err1},
           'media': {'mb': MB, 's': s2, 'mbit_s': round(MB * 8.389 / s2, 1) if e2 else 0, 'complete': bool(e2) and e2.get('complete'), 'via_proxy_mb': round((CFG['bytes'] - b0) / 1e6, 1), 'err': '' if e2 else err2}}
    row['media_event'] = e2
    rows.append(row)
    print(json.dumps(row), flush=True)
alice.kill()
print(json.dumps({'runs': rows}))
