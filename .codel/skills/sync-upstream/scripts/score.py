#!/usr/bin/env python3
"""For every grok-build commit, count codel files whose content equals
xform(upstream content) exactly.  The base commit should maximise this."""
import subprocess, os, sys, hashlib, collections, json
sys.path.insert(0, '/tmp')
from xform import xform

GB = '/Users/zhangqiong/Desktop/Project/grok-build'
CODEL = '/Users/zhangqiong/Desktop/Project/codel'


def mappath(p):
    p = p.replace('xai-grok-', 'codel-').replace('xai-', 'codel-')
    p = p.replace('grok_build', 'codel_build').replace('GrokBuild', 'CodelBuild')
    return p.replace('grok', 'codel').replace('Grok', 'Codel')


revs = [l for l in subprocess.run(['git', '-C', GB, 'rev-list', '--reverse', '--topo-order', 'HEAD'],
                                  capture_output=True, text=True).stdout.split('\n') if l]

# rev -> {mapped_path: blobsha}
trees = {}
all_blobs = set()
for r in revs:
    out = subprocess.run(['git', '-C', GB, 'ls-tree', '-r', '-z', r], capture_output=True).stdout
    d = {}
    for ent in out.split(b'\0'):
        if not ent:
            continue
        meta, path = ent.split(b'\t', 1)
        mode, typ, sha = meta.split(b' ')
        if typ != b'blob':
            continue
        p = path.decode('utf-8', 'replace')
        d[mappath(p)] = sha.decode()
        all_blobs.add(sha.decode())
    trees[r] = d

# codel files -> sha256 of content
codel = {}
for root, dirs, files in os.walk(CODEL):
    if '/.git' in root or '/target' in root:
        continue
    for f in files:
        fp = os.path.join(root, f)
        rel = os.path.relpath(fp, CODEL)
        try:
            codel[rel] = hashlib.sha256(open(fp, 'rb').read()).hexdigest()
        except OSError:
            pass

# transform each unique blob once
print(f"revs={len(revs)} unique_blobs={len(all_blobs)} codel_files={len(codel)}", flush=True)
proc = subprocess.Popen(['git', '-C', GB, 'cat-file', '--batch'],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE)
tsha = {}
blobs = sorted(all_blobs)
for i, sha in enumerate(blobs):
    proc.stdin.write((sha + '\n').encode())
    proc.stdin.flush()
    header = proc.stdout.readline().decode().split()
    size = int(header[2])
    data = proc.stdout.read(size)
    proc.stdout.read(1)  # trailing newline
    try:
        t = xform(data.decode('utf-8'))
        tsha[sha] = hashlib.sha256(t.encode()).hexdigest()
    except UnicodeDecodeError:
        tsha[sha] = None
    if i % 2000 == 0:
        print(f"  transformed {i}/{len(blobs)}", flush=True)
proc.stdin.close()

rows = []
for r in revs:
    d = trees[r]
    tot = 0
    exact = 0
    for mp, sha in d.items():
        if mp not in codel:
            continue
        tot += 1
        if tsha.get(sha) and tsha[sha] == codel[mp]:
            exact += 1
    rows.append((r, exact, tot))

subj = {}
for r in revs:
    subj[r] = subprocess.run(['git', '-C', GB, 'log', '-1', '--format=%ci', r],
                             capture_output=True, text=True).stdout.strip()[:10]

print(f"\n{'idx':>3} {'rev':>9} {'date':>10} {'exact':>6} {'total':>6} {'pct':>6}")
best = None
for i, (r, e, t) in enumerate(rows):
    pct = 100.0 * e / t if t else 0
    print(f"{i+1:>3} {r[:8]:>9} {subj[r]:>10} {e:>6} {t:>6} {pct:>5.1f}%")
    if best is None or e > best[1]:
        best = (r, e, t)
print(f"\nBEST: {best[0]} exact={best[1]}/{best[2]}")
json.dump(rows, open('/tmp/base_scores.json', 'w'))
