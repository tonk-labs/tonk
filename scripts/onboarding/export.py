#!/usr/bin/env python3
"""Convert `tonk --space product-wip export` to the bundled content snapshot.

The output uses YAML's JSON subset and Dialog's typed artifact wire format.
Keeping compiled rules and view identities preserves the source space exactly.
Credential, governance, history, and original repository identity are excluded.
"""
import base64
import pathlib
import csv
import json
import sys
from playground import PAGE, adapt_view

csv.field_size_limit(16 * 1024 * 1024)
EXCLUDED = (
    'dialog.ucan/', 'dialog.db/', 'xyz.tonk.membership/',
    'xyz.tonk.invitation/', 'xyz.tonk.invitation-execution/',
    'xyz.tonk.authorization/', 'xyz.tonk.transplant/', 'xyz.tonk.repo/',
)
TYPES = {'text': 'string', 'natural': 'uint', 'integer': 'sint'}
rows = list(csv.DictReader(sys.stdin))
private_entities = {row['of'] for row in rows if row['the'].startswith(tuple(prefix for prefix in EXCLUDED if prefix != 'dialog.db/'))}
artifacts = []
for row in rows:
    if row['the'].startswith(EXCLUDED) or row['of'] in private_entities:
        continue
    if row['as'] == 'entity' and row['is'] in private_entities:
        continue
    # product-wip always opens its startup file, but retained one reference
    # to the removed autoOpen flag. Repair the exported copy only.
    if row['the'] == 'xyz.tonk.component/module' and row['of'] == 'id:vault/component/active':
        row['is'] = row['is'].replace(
            'firstLanding && settingsRow && autoOpen && id',
            'firstLanding && settingsRow && id',
        )
    if row['the'] == 'xyz.tonk.view/ui' and row['of'] == PAGE:
        row['is'] = adapt_view(row['is'])
    artifacts.append({
        'the': row['the'], 'of': row['of'],
        'is': TYPES.get(row['as'], row['as']) + ':' + row['is'],
        'cause': None,
    })
blobs = []
for argument in sys.argv[1:]:
    entity, path = argument.split('=', 1)
    blobs.append({'entity': entity, 'data': base64.b64encode(pathlib.Path(path).read_bytes()).decode()})
json.dump({'artifacts': artifacts, 'blobs': blobs}, sys.stdout, ensure_ascii=False, indent=2)
print()
