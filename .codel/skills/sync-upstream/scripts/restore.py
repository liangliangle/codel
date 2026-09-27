#!/usr/bin/env python3
"""Materialise selected upstream crate trees into codel with path+keyword mapping."""
import os, subprocess, sys
sys.path.insert(0, '/tmp')
from xform import xform
from mappath import mappath

GB = '/Users/zhangqiong/Desktop/Project/grok-build'
CODEL = '/Users/zhangqiong/Desktop/Project/codel'
REF = 'f0e3be11'

CRATES = sys.argv[1:]

for crate in CRATES:
    out = subprocess.run(['git', '-C', GB, 'ls-tree', '-r', '-z', '--name-only', REF, '--', crate],
                         capture_output=True).stdout
    paths = [p.decode() for p in out.split(b'\0') if p]
    if not paths:
        print(f"!! not found at {REF}: {crate}")
        continue
    n = 0
    for p in paths:
        mp = mappath(p)
        data = subprocess.run(['git', '-C', GB, 'show', f'{REF}:{p}'], capture_output=True).stdout
        dest = os.path.join(CODEL, mp)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        try:
            open(dest, 'w').write(xform(data.decode('utf-8')))
        except UnicodeDecodeError:
            open(dest, 'wb').write(data)
        n += 1
    print(f"restored {crate} -> {mappath(crate)}  files={n}")
