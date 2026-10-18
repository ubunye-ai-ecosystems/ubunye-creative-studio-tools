"""E38: Sekhoto past papers on a phone with almost no data. The publisher splits a real paper into
one verified object per page. Learner A, on school Wi-Fi with no internet (LAN-only), opens 3
pages; learner B then gets a page from A (phone to phone), never from the publisher.
Pass: bytes moved per page opened, not per paper; every page verified; works with no internet.
Usage: python e38_papers.py <peer.exe> <paper.pdf> <work dir>"""
import json, os, shutil, subprocess, sys, time
from pypdf import PdfReader, PdfWriter

P, PDF, W = sys.argv[1:4]
shutil.rmtree(W, ignore_errors=True)
subprocess.run([P, 'setup', '--root', W, '--synthetic', '5', '--extra-peers', '1'], capture_output=True, check=True)
ENV = dict(os.environ, KOPANO_LAN_ONLY='1')
pages_dir = f'{W}/pages'
os.makedirs(pages_dir)
reader = PdfReader(PDF)
for i, page in enumerate(reader.pages):
    w = PdfWriter()
    w.add_page(page)
    w.compress_identical_objects()
    with open(f'{pages_dir}/p{i + 1:02d}.pdf', 'wb') as f:
        w.write(f)

def run(*args, env=None):
    r = subprocess.run([P, *args], capture_output=True, text=True, env=env, timeout=300)
    ev = [json.loads(l) for l in r.stdout.splitlines() if l.startswith('{')]
    return r.returncode, (ev[-1] if ev else None), r.stderr.strip()[-160:]

# publisher: every page is its own object; the page list (hashes) is tiny project data
hashes = [run('add-media', '--dir', f'{W}/alice', '--file', f'{pages_dir}/{n}')[1]['hash'] for n in sorted(os.listdir(pages_dir))]
paper_kb = round(os.path.getsize(PDF) / 1024)
page_kb = [round(os.path.getsize(f'{pages_dir}/{n}') / 1024) for n in sorted(os.listdir(pages_dir))]
manifest_bytes = len(json.dumps(hashes))

def serve(name):
    addr = f'{W}/{name}/addr.json'
    if os.path.exists(addr):
        os.remove(addr)
    p = subprocess.Popen([P, 'serve', '--dir', f'{W}/{name}'], env=ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not os.path.exists(addr):
        time.sleep(0.3)
    return p, addr

out = {'paper': os.path.basename(PDF), 'pages': len(hashes), 'whole_paper_kb': paper_kb, 'page_kb_min_median_max': (min(page_kb), sorted(page_kb)[len(page_kb) // 2], max(page_kb)), 'page_list_bytes': manifest_bytes}
pub, pub_addr = serve('alice')
got = 0
for i in (0, 1, 2):  # learner A opens the cover, instructions and question 1
    rc, ev, err = run('fetch-media', '--dir', f'{W}/bob', '--from', pub_addr, '--hash', hashes[i], env=ENV)
    got += ev['bytes_received'] if ev else 0
out['learner_A_3_pages_kb'] = round(got / 1024)
out['learner_A_path'] = ev and ev.get('path')
pub.kill()
# the publisher is gone; learner B gets page 3 from learner A's phone
a, a_addr = serve('bob')
rc, ev, err = run('fetch-media', '--dir', f'{W}/m0', '--from', a_addr, '--hash', hashes[2], env=ENV)
out['learner_B_from_A'] = {'ok': rc == 0 and bool(ev) and ev.get('complete'), 'kb': ev and round(ev['bytes_received'] / 1024), 'path': ev and ev.get('path')}
rc, ev, err = run('fetch-media', '--dir', f'{W}/m0', '--from', a_addr, '--hash', hashes[-1], env=ENV)
out['learner_B_page_A_lacks'] = {'refused_cleanly': rc != 0, 'error': err}
a.kill()
rc, ev, err = run('fetch-media', '--dir', f'{W}/m0', '--from', a_addr, '--hash', hashes[2], '--out', f'{W}/page3-on-B.pdf', env=ENV)
out['export_page3'] = err if rc else 'ok'
out['page_identical_to_publisher'] = os.path.exists(f'{W}/page3-on-B.pdf') and open(f'{W}/page3-on-B.pdf', 'rb').read() == open(f'{pages_dir}/p03.pdf', 'rb').read()
print(json.dumps(out))
