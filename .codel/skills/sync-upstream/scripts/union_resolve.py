#!/usr/bin/env python3
"""Resolve git-style conflict blocks by taking the union of both sides.

Suitable for manifest / list-style files (Cargo.toml, clippy.toml, JSON key
lists) where both sides' entries are needed.  Prints the files it touched.
"""
import sys
import os


def resolve(path, mode='union'):
    lines = open(path, encoding='utf-8').read().split('\n')
    out, state, ours, base, theirs = [], 0, [], [], []
    nblocks = 0
    for l in lines:
        if l.startswith('<<<<<<< CODEL(ours)'):
            state, ours, base, theirs = 1, [], [], []
            nblocks += 1
            continue
        if state == 1 and l.startswith('||||||| UPSTREAM-BASE'):
            state = 2
            continue
        if state == 2 and l.startswith('======='):
            state = 3
            continue
        if state == 3 and l.startswith('>>>>>>> UPSTREAM-HEAD'):
            seen = set(ours)
            out.extend(ours)
            for x in theirs:
                if x not in seen:
                    out.append(x)
            state = 0
            continue
        if state == 1:
            ours.append(l)
        elif state == 2:
            base.append(l)
        elif state == 3:
            theirs.append(l)
        else:
            out.append(l)
    if state != 0:
        raise SystemExit(f"unterminated conflict in {path}")
    open(path, 'w', encoding='utf-8').write('\n'.join(out))
    return nblocks


if __name__ == '__main__':
    for p in sys.argv[1:]:
        n = resolve(p)
        print(f"  resolved {n} block(s): {p}")
