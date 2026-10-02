#!/usr/bin/env python3
"""
Sync grok-build -> codel.

For every file changed upstream between BASE and HEAD, perform a 3-way merge onto
the codel working tree, translating grok-build paths and identifiers into codel
terms:

    ancestor = xform(grok-build @ BASE  file)
    ours     = codel working-tree file
    theirs   = xform(grok-build @ HEAD  file)

where xform renames xai-*/xai_*/grok*/x.ai/grok.com to codel equivalents.

Clean merges are written back automatically.  Conflicts are written with
git-style markers plus a sibling `.sync-conflict` file carrying the ancestor and
theirs for reference, and recorded in the report.
"""
import os
import subprocess
import sys
import json

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xform import xform
from mappath import mappath

GB = '/Users/zhangqiong/Desktop/Project/grok-build'
CODEL = '/Users/zhangqiong/Desktop/Project/codel'
REPORT = '/tmp/sync_report.json'

# Whole crates the fork removed: never (re)create files under these.
REMOVED_CRATES = [
    'xai-grok-telemetry', 'xai-mixpanel', 'xai-tracing', 'xai-compaction-transcript',
    'xai-fuzzy-file-search', 'xai-grok-active-sessions', 'xai-grok-bundle',
    'xai-grok-diag-server', 'xai-grok-extra-ca', 'xai-grok-foreign-sessions',
    'xai-grok-home', 'xai-grok-pager-diff', 'xai-grok-session-events',
    'xai-grok-session-search', 'xai-grok-workspace-daemon',
]
# Always excluded from the sync.
EXCLUDE_EXACT = {'SOURCE_REV', 'Cargo.lock'}


def git_show(rev, path):
    r = subprocess.run(['git', '-C', GB, 'show', f'{rev}:{path}'], capture_output=True)
    if r.returncode != 0:
        return None
    return r.stdout


def ls_tree(rev):
    out = subprocess.run(['git', '-C', GB, 'ls-tree', '-r', '-z', '--name-only', rev],
                         capture_output=True, text=True).stdout
    return [x for x in out.split('\0') if x]


def is_excluded(p):
    if p in EXCLUDE_EXACT:
        return True
    if any(('/' + c + '/') in '/' + p for c in REMOVED_CRATES):
        return True
    return False


def is_text(b):
    if b is None:
        return False
    try:
        b.decode('utf-8')
        return True
    except UnicodeDecodeError:
        return False


def merge_file(ours, ancestor, theirs):
    """Return (merged_bytes, nconflicts)."""
    open('/tmp/.mg_ours', 'wb').write(ours)
    open('/tmp/.mg_anc', 'wb').write(ancestor)
    open('/tmp/.mg_theirs', 'wb').write(theirs)
    r = subprocess.run(['git', 'merge-file', '-p', '--diff3',
                        '-L', 'CODEL(ours)', '-L', 'UPSTREAM-BASE', '-L', 'UPSTREAM-HEAD',
                        '/tmp/.mg_ours', '/tmp/.mg_anc', '/tmp/.mg_theirs'],
                       capture_output=True)
    return r.stdout, (r.returncode if r.returncode >= 0 else -1)


def main():
    base, head = sys.argv[1], sys.argv[2]
    dry = '--dry-run' in sys.argv
    base_files = set(ls_tree(base))
    head_files = set(ls_tree(head))
    all_files = sorted((base_files | head_files))

    codel_files = subprocess.run(['git', '-C', CODEL, 'ls-files'],
                                 capture_output=True, text=True).stdout.split('\n')
    codel_files = {c for c in codel_files if c}

    actions = {'created': [], 'updated': [], 'unchanged': [], 'deleted': [],
               'conflict': [], 'skipped-removed': [], 'skipped-absent': [],
               'skipped-binary-conflict': []}

    for p in all_files:
        if is_excluded(p):
            if p not in EXCLUDE_EXACT:
                actions['skipped-removed'].append(p)
            continue
        mp = mappath(p)
        cp = os.path.join(CODEL, mp)
        existed_base = p in base_files
        exists_head = p in head_files
        in_codel = os.path.exists(cp)

        anc = git_show(base, p) if existed_base else b''
        the = git_show(head, p) if exists_head else b''

        # codel deliberately removed this file -> keep it removed
        if existed_base and not in_codel:
            actions['skipped-absent'].append(mp)
            continue
        # brand new upstream file
        if not existed_base and not in_codel:
            if not exists_head:
                continue
            if not is_text(the):
                continue
            os.makedirs(os.path.dirname(cp), exist_ok=True)
            if not dry:
                open(cp, 'wb').write(xform(the.decode('utf-8')).encode('utf-8'))
            actions['created'].append(mp)
            continue
        # upstream deleted the file
        if existed_base and not exists_head:
            if not in_codel:
                continue
            ours = open(cp, 'rb').read()
            if ours == anc:
                if not dry:
                    os.remove(cp)
                actions['deleted'].append(mp)
            else:
                actions['conflict'].append({'path': mp, 'kind': 'upstream-deleted'})
            continue

        ours = open(cp, 'rb').read()
        if not is_text(anc) or not is_text(the) or not is_text(ours):
            if ours == anc:
                if not dry:
                    open(cp, 'wb').write(the)
                actions['updated'].append(mp)
            elif ours == the:
                actions['unchanged'].append(mp)
            else:
                actions['skipped-binary-conflict'].append(mp)
            continue

        A, T, O = xform(anc.decode('utf-8')), xform(the.decode('utf-8')), ours.decode('utf-8')
        if A == T:
            actions['unchanged'].append(mp)
            continue
        if O == A:
            if not dry:
                open(cp, 'w').write(T)
            actions['updated'].append(mp)
            continue
        if O == T:
            actions['unchanged'].append(mp)
            continue
        merged, nconf = merge_file(O.encode(), A.encode(), T.encode())
        if not dry:
            open(cp, 'wb').write(merged)
            if nconf > 0:
                # stash upstream HEAD's transformed version for the resolver
                os.makedirs('/tmp/conflict_theirs', exist_ok=True)
                tpath = os.path.join('/tmp/conflict_theirs', mp.replace('/', '__'))
                open(tpath, 'w').write(T)
        if nconf > 0:
            actions['conflict'].append({'path': mp, 'kind': 'merge', 'conflicts': nconf})
        else:
            actions['updated'].append(mp)

    for k in actions:
        print(f"{k}: {len(actions[k])}")
    json.dump(actions, open(REPORT, 'w'), indent=1)
    print("report:", REPORT)


if __name__ == '__main__':
    main()
