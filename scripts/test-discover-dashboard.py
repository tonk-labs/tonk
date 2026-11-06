#!/usr/bin/env python3
"""Exercise the saved SQL on inline synthetic rows, without ingesting events."""
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
module = importlib.util.spec_from_file_location('discover_dashboard', ROOT / 'scripts/posthog-discover-dashboard.py')
dashboard = importlib.util.module_from_spec(module)
module.loader.exec_module(dashboard)
spec = json.loads(dashboard.SPEC.read_text())
labels = {item['slug']: item for item in spec['template_labels']}
rows = []


def event(name, profile, space, date, template='', identity='profile', environment='production'):
    values = {'event': name, 'distinct_id': 'tonk:' + profile * 64,
              'space_key': space * 16, 'template_key': template,
              'conversion': 'created' if name == 'space_conversion' else '',
              'identity_state': identity, 'environment': environment}
    columns = ["'" + value.replace("'", "''") + "' AS " + key for key, value in values.items()]
    columns.append(f"toDateTime('{date} 12:00:00') AS timestamp")
    rows.append('SELECT ' + ', '.join(columns))


k = labels['kanoodel']['template_id']
l = labels['little-writer']['template_id']
for profile, space, date, template in [('a','1','2026-09-20',k), ('a','1','2026-09-20',k),
    ('b','2','2026-09-20',k), ('a','3','2026-09-21',k), ('c','4','2026-09-30',l),
    ('d','5','2026-09-27','f'*16)]:
    event('space_conversion',profile,space,date,template)
event('space_conversion','e','6','2026-09-20',k,environment='staging')
for profile, space, date in [('a','1','2026-09-20'), ('a','1','2026-09-21'),
    ('a','1','2026-09-21'), ('a','3','2026-09-22'), ('b','2','2026-09-28'),
    ('c','4','2026-10-01'), ('d','5','2026-09-28'), ('c','1','2026-09-22'),
    ('f','9','2026-09-22')]:
    event('space_entered',profile,space,date)
event('space_entered','e','1','2026-09-23',identity='unresolved')
fixture = 'fixture_events AS (' + ' UNION ALL '.join(rows) + '), '


def query(sql):
    sql = sql.replace('FROM events', 'FROM fixture_events')
    for key, column in [('space_id','space_key'), ('template_id','template_key'),
                        ('conversion','conversion'), ('identity_state','identity_state'), ('environment','environment')]:
        sql = sql.replace('properties.' + key, column)
    sql = sql.replace('now()', "toDateTime('2026-10-01 12:00:00')")
    result = dashboard.api('execute-sql', {'query': sql.replace('WITH ', 'WITH ' + fixture, 1)})
    lines = result.splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(('template|','all_space_entries|')))
    headers = lines[start].split('|')
    return [dict(zip(headers,line.split('|'))) for line in lines[start+1:] if '|' in line]


results = [query(insight['query']) for insight in spec['insights']]
adoption, reach, frequency, retention = [{row['template']:row for row in result} for result in results[:4]]
name = labels['kanoodel']['name']
assert adoption[name]['created_spaces'] == '3', adoption
assert adoption[name]['identified_creators'] == '2', adoption
assert reach[name]['visiting_profiles'] == '3', reach
assert reach[name]['visited_spaces'] == '3', reach
assert reach[name]['visiting_collaborators'] == '1', reach
assert frequency[name]['profile_days'] == '5', frequency
assert frequency[name]['profile_space_days'] == '5', frequency
assert frequency[name]['repeat_profiles'] == '1', frequency
assert retention[name]['creator_cohort'] == '2', retention
for key in ('d1_eligible','d1_7_eligible'):
    assert retention[name][key] == '2', retention
for key in ('d1_percent','d1_7_percent'):
    assert float(retention[name][key]) == 50, retention
immature = retention[labels['little-writer']['name']]
assert immature['d1_eligible'] == '0' and immature['d1_7_eligible'] == '0', immature
assert immature['d1_percent'] in ('null','NULL','(null)',''), immature
assert immature['d1_7_percent'] in ('null','NULL','(null)',''), immature
partial = retention['Unlisted template ' + 'f'*16]
assert float(partial['d1_percent']) == 100, partial
assert partial['d1_7_percent'] in ('null','NULL','(null)',''), partial
coverage = results[4][0]
for key, expected in [('all_space_entries','10'), ('template_attributed_entries','9'),
                      ('unattributed_entries','1'), ('attributed_identified_entries','8')]:
    assert coverage[key] == expected, coverage
print('All five SQL fixture checks passed: deduplication, creators/collaborators, day-zero/day-eight exclusion, immature cohorts, unknown templates, and attribution coverage.')
