#!/usr/bin/env python3
"""Regenerate/check the single-crate vendor from a patched pinned Dialog checkout."""
import argparse
import json
from pathlib import Path
import re
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('dialog', type=Path)
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent
source = args.dialog / 'rust/dialog-remote-ucan-s3'
output = repo / 'vendor/dialog-remote-ucan-s3'
workspace = tomllib.loads((args.dialog / 'Cargo.toml').read_text())['workspace']

def value(item):
    if isinstance(item, dict):
        return '{ ' + ', '.join(f'{k} = {value(v)}' for k, v in item.items()) + ' }'
    return json.dumps(item)

text = (source / 'Cargo.toml').read_text()
for key, item in workspace['package'].items():
    text = text.replace(f'{key}.workspace = true', f'{key} = {value(item)}')

def dependency(match):
    name = match[1]
    local = tomllib.loads('entry = {' + match[2] + '}')['entry']
    local.pop('workspace')
    inherited = workspace['dependencies'][name]
    inherited = {'version': inherited} if isinstance(inherited, str) else dict(inherited)
    if 'path' in inherited:
        inherited.pop('path')
        inherited.update(git='https://github.com/dialog-db/dialog-db.git', tag='tonk-2026-09-14')
    features = inherited.pop('features', []) + local.pop('features', [])
    inherited.update(local)
    if features:
        inherited['features'] = list(dict.fromkeys(features))
    return f'{name} = {value(inherited)}'

text = re.sub(r'(?m)^([\w-]+) = \{ ([^\n]*workspace = true[^\n]*) \}$', dependency, text)
expected = {p.relative_to(source): p.read_bytes() for p in source.rglob('*') if p.is_file()}
expected[Path('Cargo.toml')] = text.encode()
expected[Path('LICENSE')] = (args.dialog / 'LICENSE').read_bytes()
if args.check:
    # Provenance is the only Tonk-authored file outside the generated set.
    # Reject stale/untracked sources (including auto-discovered build.rs/tests)
    # and symlinks rather than validating only the files we expected to find.
    allowed = set(expected) | {Path('PROVENANCE.md')}
    for entry in sorted(output.rglob('*')):
        relative = entry.relative_to(output)
        if entry.is_symlink():
            raise SystemExit(f'vendor contains symlink: {entry}')
        if not entry.is_dir() and relative not in allowed:
            raise SystemExit(f'vendor contains unexpected file: {entry}')
for relative, content in expected.items():
    target = output / relative
    if args.check:
        if not target.exists() or target.read_bytes() != content:
            raise SystemExit(f'vendor differs: {target}')
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
print(f'{"checked" if args.check else "generated"} {len(expected)} vendored files')
