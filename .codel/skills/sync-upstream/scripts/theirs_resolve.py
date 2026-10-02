#!/usr/bin/env python3
"""Resolve git-style conflict blocks by adopting the UPSTREAM-HEAD side.

This is the sync direction: the file becomes upstream's newer content (already
renamed into codel terms by the merge), with codel's own edits kept wherever the
two sides did not overlap.  Fails loudly on an unterminated block.
"""
import sys


def resolve(path):
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
            out.extend(theirs)
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
    total = 0
    for p in sys.argv[1:]:
        n = resolve(p)
        total += n
        print(f"  {n:3d} block(s): {p}")
    print(f"resolved {total} block(s) across {len(sys.argv) - 1} file(s)")
