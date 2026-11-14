"""Offline checks for docs/analytics/interface-dashboards.json; PostHog is not contacted.

`scripts/posthog-interface-dashboards.py validate` runs the queries against PostHog.
"""
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = json.loads((ROOT / 'docs/analytics/interface-dashboards.json').read_text())
PROFILE = "match(distinct_id, '^tonk:[0-9a-f]{64}$')"
# Each interface dashboard's scope, as its queries must state it.
SCOPES = {
    'Web app': ('Web: ', "properties.environment IN ('production','staging','dev')"
                         " AND coalesce(properties.shell, '') != 'desktop'"),
    'CLI': ('CLI: ', "properties.environment = 'cli'"),
    'Desktop app': ('Desktop: ', "properties.environment = 'desktop'"),
}
FORBIDDEN = ('$current_url', '$pathname', '$referrer', 'properties.route', 'properties.email',
             'properties.space_id', 'person.properties')
# The overview's interface: web app views embedded in the desktop app count as desktop.
INTERFACE = ("multiIf(properties.environment = 'cli', 'cli', properties.environment = 'desktop'"
             " OR (properties.environment = 'production' AND coalesce(properties.shell, '') = 'desktop'),"
             " 'desktop', properties.environment = 'production', 'web', '')")
DISPLAYS = {'ActionsTable', 'ActionsLineGraph', 'ActionsStackedBar'}


def boards():
    return {board['dashboard']['name']: board for board in SPEC['dashboards']}


def product_actions():
    source = (ROOT / 'rust/tonk-analytics/src/product.rs').read_text()
    body = re.search(r'ProductAction \{(.*?)\n    \}', source, re.S).group(1)
    names = re.findall(r'\b[A-Z][A-Za-z]+\b', re.sub(r'//[^\n]*', '', body))
    return {re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower() for name in names}


class InterfaceDashboardsTest(unittest.TestCase):
    def test_there_is_an_overview_and_one_dashboard_per_interface(self):
        self.assertEqual(set(boards()), {'Interfaces overview', *SCOPES})

    def test_insight_names_are_unique_and_prefixed_by_dashboard(self):
        names = [insight['name'] for board in SPEC['dashboards'] for insight in board['insights']]
        self.assertEqual(len(names), len(set(names)))
        for title, (prefix, _) in {**SCOPES, 'Interfaces overview': ('Interfaces: ', '')}.items():
            for insight in boards()[title]['insights']:
                self.assertTrue(insight['name'].startswith(prefix), insight['name'])

    def test_each_interface_dashboard_stays_inside_its_interface(self):
        for title, (_, scope) in SCOPES.items():
            for insight in boards()[title]['insights']:
                self.assertIn(scope, insight['query'], insight['name'])

    def test_the_overview_counts_production_web_only(self):
        for insight in boards()['Interfaces overview']['insights']:
            self.assertNotIn("'staging'", insight['query'], insight['name'])
            self.assertNotIn("'dev'", insight['query'], insight['name'])

    def test_the_overview_counts_embedded_web_views_as_desktop(self):
        for insight in boards()['Interfaces overview']['insights']:
            if 'multiIf(' in insight['query']:
                self.assertIn(INTERFACE, insight['query'], insight['name'])

    def test_profile_counts_use_complete_profile_hashes(self):
        for board in SPEC['dashboards']:
            for insight in board['insights']:
                if 'distinct_id' in insight['query']:
                    self.assertIn(PROFILE, insight['query'], insight['name'])

    def test_queries_are_read_only_and_reviewed(self):
        for board in SPEC['dashboards']:
            for insight in board['insights']:
                query = insight['query']
                self.assertTrue(query.startswith('SELECT '), insight['name'])
                self.assertNotIn(';', query, insight['name'])
                self.assertIn(insight['display'], DISPLAYS, insight['name'])
                self.assertTrue(insight['description'], insight['name'])
                for field in FORBIDDEN:
                    self.assertNotIn(field, query, insight['name'])
                for group in re.findall(r'GROUP BY (.*?)(?: ORDER BY| HAVING|\)|$)', query):
                    self.assertNotIn(' AS ', group, insight['name'])

    def test_outcome_cohorts_read_terminal_events_through_now(self):
        # The ten-minute maturity rule applies to the start, never to the terminal receipt.
        for board in SPEC['dashboards']:
            for insight in board['insights']:
                query = insight['query']
                if 'INTERVAL 10 MINUTE' in query:
                    self.assertIn("minIf(timestamp, properties.phase = 'started') < now() - INTERVAL 10 MINUTE", query, insight['name'])
                    self.assertNotIn('timestamp < now() - INTERVAL 10 MINUTE', query, insight['name'])

    def test_every_web_tile_keeps_environments_apart(self):
        for insight in boards()['Web app']['insights']:
            self.assertRegex(insight['query'], r'GROUP BY [^)]*\benvironment\b', insight['name'])

    def test_named_actions_exist_in_the_product_vocabulary(self):
        known = product_actions()
        for board in SPEC['dashboards']:
            for insight in board['insights']:
                for clause in re.findall(r"properties\.action (?:IN \(([^)]*)\)|= '([a-z_]+)')", insight['query']):
                    for name in re.findall(r"'([a-z_]+)'", clause[0]) or [clause[1]]:
                        self.assertIn(name, known, insight['name'])


if __name__ == '__main__':
    unittest.main()
