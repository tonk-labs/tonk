#!/usr/bin/env python3
"""Validate and publish the dedicated Discover template dashboard."""
import argparse
import datetime as dt
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / 'docs/analytics/discover-dashboard.json'
module = importlib.util.spec_from_file_location('product_dashboard', ROOT / 'scripts/posthog-product-dashboard.py')
product = importlib.util.module_from_spec(module)
module.loader.exec_module(product)
api = product.api


def without_nulls(value):
    if isinstance(value, dict):
        return {key: without_nulls(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [without_nulls(item) for item in value]
    return value


def validate(spec):
    project = api('project-get', {})
    if int(project['id']) != spec['project_id']:
        raise SystemExit('Wrong active PostHog project; refusing to proceed')
    for insight in spec['insights']:
        result = api('execute-sql', {'query': insight['query']})
        if isinstance(result, dict) and result.get('error'):
            raise SystemExit(str(result['error']))
        # The CLI currently returns formatted text including taxonomy warnings.
        # An absent new event is expected; an execution error is not.
        if isinstance(result, str) and any(marker in result.lower() for marker in ('query failed', 'error executing', 'exception:', 'error:')):
            raise SystemExit(result)
        print(f"Validated {insight['name']}\n{result}", flush=True)


def publish(spec):
    dashboard = product.exact_dashboard(spec['dashboard']['name'])
    if dashboard is None:
        dashboard = api('dashboard-create', spec['dashboard'])
    else:
        dashboard = api('dashboard-update', {'id': dashboard['id'], **spec['dashboard']})
    dashboard_id = int(dashboard['id'])
    published = []
    for insight in spec['insights']:
        current = product.exact_insight(insight['name'])
        body = {key: insight[key] for key in ('name', 'description')}
        body.update(query=product.query_node(insight), tags=['product', 'discover', 'privacy-reviewed'])
        # Preserve any other dashboard memberships on subsequent updates.
        if current is not None:
            current = api('insight-get', {'id': current['short_id']})
        memberships = (current or {}).get('dashboards', [])
        memberships += [tile['dashboard_id'] for tile in (current or {}).get('dashboard_tiles', []) if not tile.get('deleted')]
        body['dashboards'] = sorted(set(memberships + [dashboard_id]))
        saved = api('insight-update', {'id': current['short_id'], **body}) if current else api('insight-create', body)
        verified = api('insight-get', {'id': saved['short_id']})
        for key in ('name', 'description', 'query'):
            if without_nulls(verified.get(key)) != without_nulls(body[key]):
                raise SystemExit(f'Read-back mismatch: {insight["name"]}: {key}')
        published.append({'name': insight['name'], 'id': verified['id'], 'short_id': verified['short_id']})
        print(f"Published {insight['name']}: {verified['short_id']}", flush=True)
    verified = api('dashboard-get', {'id': dashboard_id})
    tiles = {tile['insight']['short_id']: tile['id'] for tile in verified['tiles'] if tile.get('insight')}
    order = [tiles[item['short_id']] for item in published]
    # Retain unexpected user-added tiles at the end.
    order += [tile['id'] for tile in verified['tiles'] if tile['id'] not in order]
    api('dashboard-reorder-tiles', {'id': dashboard_id, 'tile_order': order, 'layout': 'full_width'})
    verified = api('dashboard-get', {'id': dashboard_id})
    if [tile['id'] for tile in sorted(verified['tiles'], key=lambda tile: tile.get('order', 0))] != order:
        raise SystemExit('Dashboard tile order read-back mismatch')
    if verified['description'] != spec['dashboard']['description']:
        raise SystemExit('Dashboard description read-back mismatch')
    spec['published'] = {'dashboard_id': dashboard_id, 'verified_at': dt.datetime.now(dt.UTC).isoformat(), 'insights': published}
    SPEC.write_text(json.dumps(spec, indent=2) + '\n')
    print(f'Verified dashboard {dashboard_id}: {len(published)} insights', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['validate', 'publish'])
    args = parser.parse_args()
    spec = json.loads(SPEC.read_text())
    validate(spec)
    if args.mode == 'publish':
        publish(spec)
