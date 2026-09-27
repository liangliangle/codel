#!/usr/bin/env python3
"""Iteratively delete test functions in codel-login that no longer compile.

A test that references the removed login/OIDC/refresh machinery is gone with it;
tests of retained behaviour keep compiling and are left alone.  Only functions
carrying `#[test]` / `#[tokio::test]` are deleted — anything else that errors is
reported for a human.
"""
import os
import re
import subprocess
import sys

ROOT = '/Users/zhangqiong/Desktop/Project/codel'
PKG = 'codel-login'
CRATE_DIR = os.path.join(ROOT, 'crates/codegen/codel-login')


def check():
    env = dict(os.environ, PROTOC=os.path.join(ROOT, 'bin/protoc'))
    r = subprocess.run(['cargo', 'check', '-p', PKG, '--all-targets',
                        '--message-format=short'],
                       cwd=ROOT, capture_output=True, text=True, env=env)
    return r.stdout + r.stderr


def error_locations(out):
    locs = set()
    for m in re.finditer(r'^(crates/codegen/codel-login/src/[^:]+):(\d+):\d+: error', out, re.M):
        locs.add((m.group(1), int(m.group(2))))
    return locs


def enclosing_test_fn(path, line):
    """Return (start, end, name) of the innermost test-only item containing `line`.

    Handles `#[test]` fns, and any `fn`/`struct`/`impl` inside a test-only file or
    a `#[cfg(test)] mod tests` block, so fake refreshers and other test scaffolding
    left behind by a removal are pruned with the tests that used them.
    """
    text = open(path, encoding='utf-8').read()
    lines = text.split('\n')
    test_only_file = path.endswith(('_tests.rs', '/tests.rs'))

    i = min(line - 1, len(lines) - 1)
    while i >= 0 and not re.match(r'\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|impl)\b', lines[i]):
        i -= 1
    if i < 0:
        return None
    m = re.match(r'\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|impl)\s+(\w+)', lines[i])
    name = m.group(1) if m else f'<line {i + 1}>'

    # attribute / doc block immediately above
    j = i - 1
    is_test = False
    while j >= 0 and lines[j].strip().startswith(('#[', '///', '//')):
        if re.match(r'\s*#\[(?:tokio::)?test\]', lines[j]):
            is_test = True
        j -= 1

    if not is_test and not test_only_file:
        # allow items inside a `#[cfg(test)] mod tests` block
        k = j
        in_cfg_test_mod = False
        while k >= 0:
            if re.match(r'\s*mod\s+tests\s*\{', lines[k]):
                if k > 0 and re.match(r'\s*#\[cfg\(test\)\]', lines[k - 1]):
                    in_cfg_test_mod = True
                    break
            k -= 1
        if not in_cfg_test_mod:
            return None

    # span by brace matching from the declaration line
    depth = 0
    started = False
    k = i
    while k < len(lines):
        depth += lines[k].count('{') - lines[k].count('}')
        if '{' in lines[k]:
            started = True
        if started and depth == 0:
            break
        k += 1
    if k >= len(lines):
        return None

    start = i
    while start - 1 >= 0 and lines[start - 1].strip().startswith(('#[', '///', '//')):
        start -= 1
    return start, k, name


def balance(text):
    d = 0
    for c in text:
        if c == '{':
            d += 1
        elif c == '}':
            d -= 1
    return d


def delete_span(full, start, end):
    """Delete lines [start, end] only if the file's brace balance is preserved."""
    text = open(full, encoding='utf-8').read()
    before = balance(text)
    lines = text.split('\n')
    del lines[start:end + 1]
    after = balance('\n'.join(lines))
    if after != before:
        return False
    open(full, 'w', encoding='utf-8').write('\n'.join(lines))
    return True


def main():
    for _ in range(60):
        out = check()
        locs = error_locations(out)
        if not locs:
            print('codel-login test targets compile')
            return 0
        removed = []
        skipped = []
        by_file = {}
        for path, line in locs:
            by_file.setdefault(path, []).append(line)
        for path, line_list in by_file.items():
            full = os.path.join(ROOT, path)
            for line in sorted(line_list, reverse=True):
                fn = enclosing_test_fn(full, line)
                if not fn:
                    skipped.append(f'{path}:{line}')
                    continue
                start, end, name = fn
                if delete_span(full, start, end):
                    removed.append(f'{path}:{name}')
                else:
                    skipped.append(f'{path}:{name} (balance)')
        print(f'pass: removed {len(removed)} test(s); non-test errors: {len(set(skipped))}')
        if not removed:
            print('no test fn removed this pass; remaining errors need manual work:')
            for s in sorted(set(skipped))[:40]:
                print('   ', s)
            return 1
    print('did not converge')
    return 1


if __name__ == '__main__':
    sys.exit(main())
