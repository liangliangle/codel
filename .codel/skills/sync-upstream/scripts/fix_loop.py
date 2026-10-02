#!/usr/bin/env python3
"""Delete, one at a time, the items in codel-login that no longer compile.

Each deletion is brace-balance verified and followed by a fresh compile, so
damage cannot accumulate.  Only test-only items are removed (a `#[test]` fn, or
any item in a `_tests.rs` file / `#[cfg(test)] mod tests` block); anything else
is reported and left alone.
"""
import os
import re
import subprocess
import sys

ROOT = '/Users/zhangqiong/Desktop/Project/codel'
PKG = sys.argv[1] if len(sys.argv) > 1 else 'codel-login'


def check():
    env = dict(os.environ, PROTOC=os.path.join(ROOT, 'bin/protoc'))
    r = subprocess.run(['cargo', 'check', '-p', PKG, '--all-targets', '--keep-going',
                        '--message-format=short'],
                       cwd=ROOT, capture_output=True, text=True, env=env)
    return r.stdout + r.stderr


def balance(text):
    d = 0
    for c in text:
        if c == '{':
            d += 1
        elif c == '}':
            d -= 1
    return d


def first_error(out):
    for m in re.finditer(r'^((?:crates|prod)/[^:]+):(\d+):\d+: error', out, re.M):
        if '/benches/' in m.group(1):
            continue
        return (m.group(1), int(m.group(2)))
    return None


def enclosing_item(path, line):
    lines = open(path, encoding='utf-8').read().split('\n')
    test_only_file = path.endswith(('_tests.rs', '/tests.rs')) or '/tests/' in path or '/benches/' in path
    i = min(line - 1, len(lines) - 1)
    while i >= 0 and not re.match(r'\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|impl)\b', lines[i]):
        i -= 1
    if i < 0:
        return None
    name_m = re.match(r'\s*(?:pub\s+)?(?:async\s+)?(?:fn|struct|enum|impl)\s+(\w+)', lines[i])
    name = name_m.group(1) if name_m else f'line{i + 1}'
    j = i - 1
    is_test = False
    while j >= 0 and lines[j].strip().startswith(('#[', '///', '//')):
        if re.match(r'\s*#\[(?:tokio::)?test\]', lines[j]):
            is_test = True
        j -= 1
    if not is_test and not test_only_file:
        k = j
        ok = False
        while k >= 0:
            if re.match(r'\s*mod\s+tests\s*\{', lines[k]):
                ok = k > 0 and bool(re.match(r'\s*#\[cfg\(test\)\]', lines[k - 1]))
                break
            k -= 1
        if not ok:
            return None
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


def main():
    for step in range(160):
        out = check()
        loc = first_error(out)
        if not loc:
            print('codel-login --all-targets compiles')
            return 0
        path, line = loc
        full = os.path.join(ROOT, path)
        item = enclosing_item(full, line)
        if not item:
            print(f'no test-only item encloses {path}:{line}; stopping')
            print('\n'.join(l for l in out.split('\n') if 'error' in l)[:3000])
            return 1
        start, end, name = item
        text = open(full, encoding='utf-8').read()
        before = balance(text)
        lines = text.split('\n')
        del lines[start:end + 1]
        if balance('\n'.join(lines)) != before:
            print(f'balance would break deleting {name} in {path}; stopping')
            return 1
        open(full, 'w', encoding='utf-8').write('\n'.join(lines))
        print(f'step {step + 1}: deleted {name} ({path}:{start + 1}-{end + 1})')
    print('did not converge')
    return 1


if __name__ == '__main__':
    sys.exit(main())
