"""E25: what the relay sees. A recording proxy sits in front of a local iroh-relay; Alice serves
and Bob pulls relay-only, so every byte passes the relay. Then we search the recording for things
that should be private (project name, photo names, people, edit values) and for things that are
visible by design (device endpoint ids).
Usage: python e25_relay_view.py <peer.exe> <iroh-relay.exe> <work dir>"""
import json, os, shutil, socket, subprocess, sys, threading, time

P, RELAY, W = sys.argv[1:4]
shutil.rmtree(W, ignore_errors=True)
os.makedirs(W)
captured = bytearray()
lock = threading.Lock()

def pump(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            with lock:
                captured.extend(d)
            b.sendall(d)
    except OSError:
        pass
    for s in (a, b):
        try:
            s.close()
        except OSError:
            pass

def proxy():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(('127.0.0.1', 4000))
    srv.listen(16)
    while True:
        c, _ = srv.accept()
        u = socket.create_connection(('::1', 3340))
        threading.Thread(target=pump, args=(c, u), daemon=True).start()
        threading.Thread(target=pump, args=(u, c), daemon=True).start()

relay = subprocess.Popen([RELAY, '--dev'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
threading.Thread(target=proxy, daemon=True).start()
time.sleep(2)
env = dict(os.environ, KOPANO_RELAY_URL='http://127.0.0.1:4000')
subprocess.run([P, 'setup', '--root', W, '--synthetic', '200'], capture_output=True)
# a distinctive edit Alice makes before Bob pulls
subprocess.run([P, 'edit', '--dir', f'{W}/alice', '--key', 'IMG_0000007.ARW', '--field', 'develop.exposure', '--value', '1.2345678'], capture_output=True, env=env)
a = subprocess.Popen([P, 'serve', '--dir', f'{W}/alice', '--relay-only'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
while not os.path.exists(f'{W}/alice/addr.json'):
    time.sleep(0.3)
r = subprocess.run([P, 'pull', '--dir', f'{W}/bob', '--from', f'{W}/alice/addr.json', '--relay-only'], capture_output=True, text=True, env=env, timeout=120)
pull = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
time.sleep(1)
a.kill()
relay.kill()
cert = json.load(open(f'{W}/alice/cert.json'))
addr = json.load(open(f'{W}/alice/addr.json'))
with lock:
    blob = bytes(captured)

def seen(s):
    return s.encode() in blob

private = {
    'project name': 'Wedding shoot',
    'photo file name': 'IMG_0000007.ARW',
    'edit value': '1.2345678',
    'person name': cert['body']['name'],
    'account id': cert['body']['account'],
    'device label': cert['body'].get('device_label', 'lab'),
    'setting name': 'exposure',
}
endpoint = addr.get('id') or addr.get('node_id') or ''
by_design = {'Alice endpoint id (hex)': endpoint}
print(json.dumps({
    'pull_ok': bool(pull) and pull[-1].get('result', '').startswith(('fast-forward', 'merged', 'already')),
    'pull_path': pull[-1].get('path') if pull else None,
    'relay_bytes_seen': len(blob),
    'private_found_in_relay_traffic': {k: seen(v) for k, v in private.items()},
    'visible_by_design': {k: (seen(v) or (bytes.fromhex(v) in blob if len(v) == 64 else False)) for k, v in by_design.items()},
}))
