#!/usr/bin/env python3
"""Validate/publish explicit metric corrections without replacing memberships."""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / "docs/analytics/metrics-dashboard.json"
module_spec = importlib.util.spec_from_file_location(
    "product_dashboard", ROOT / "scripts/posthog-product-dashboard.py"
)
product = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(product)
api = product.api


def query(sql: str) -> dict:
    return {
        "kind": "DataVisualizationNode",
        "source": {"kind": "HogQLQuery", "query": sql},
        "display": "ActionsTable",
    }


def without_nulls(value):
    """PostHog drops unset query fields on write; preserve every non-null value."""
    if isinstance(value, dict):
        return {key: without_nulls(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [without_nulls(item) for item in value]
    return value


def arrange_dashboard(layout: dict) -> None:
    dashboard = api("dashboard-get", {"id": layout["id"]})
    tiles = {tile["insight"]["short_id"]: tile for tile in dashboard["tiles"]
             if tile.get("insight")}
    if not set(layout["insights"]).issubset(tiles):
        raise SystemExit("cannot arrange dashboard: a required insight is missing")
    # Only remove explicitly reviewed tiles, never an unexpected user addition.
    # Soft deletion preserves the saved insight and all other dashboards.
    for identifier in layout.get("remove", []):
        if identifier in tiles:
            api("dashboard-delete-tile", {
                "id": layout["id"], "tile_id": tiles[identifier]["id"],
            }, confirm=True)
    order = [tiles[identifier]["id"] for identifier in layout["insights"]]
    api("dashboard-reorder-tiles", {
        "id": layout["id"], "tile_order": order, "layout": "two_column",
    })
    verified = api("dashboard-get", {"id": layout["id"]})
    actual = sorted(verified["tiles"], key=lambda tile: tile.get("order", 0))
    if [tile["id"] for tile in actual] != order:
        raise SystemExit("dashboard tile order read-back mismatch")
    for index, tile in enumerate(actual):
        grid = tile["layouts"]["sm"]
        expected = {"x": index % 2 * 6, "y": index // 2 * 5, "w": 6, "h": 5}
        if any(grid.get(key) != value for key, value in expected.items()):
            raise SystemExit("dashboard layout read-back mismatch")
    print(f"verified layout: {layout['id']} ({len(order)} tiles)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["validate", "publish"])
    parser.add_argument("--only", action="append", help="Only process this exact insight name (repeatable)")
    args = parser.parse_args()
    spec = json.loads(SPEC.read_text())
    selected = [item for item in spec["insights"] if not args.only or item["name"] in args.only]
    if args.only and set(args.only) - {item["name"] for item in selected}:
        raise SystemExit("unknown insight name in --only")
    # Validate SQL queries before any mutation. Existing memberships are never
    # sent in an update: PostHog interprets that field as a full replacement.
    for item in selected:
        if "sql" in item:
            result = api("execute-sql", {"query": item["sql"]})
            if isinstance(result, dict) and result.get("error"):
                raise SystemExit(f"query failed: {item['name']}: {result['error']}")
        label = "validated SQL" if "sql" in item else "prepared definition"
        print(f"{label}: {item['name']}")
    if args.mode != "publish":
        return
    for item in selected:
        current = (api("insight-get", {"id": item["id"]}) if item.get("id")
                   else product.exact_insight(item["name"]))
        body = {"name": item["name"], "description": item["description"]}
        if "sql" in item:
            body["query"] = query(item["sql"])
            body["query"]["display"] = item.get("display", "ActionsTable")
            if "chart_settings" in item:
                body["query"]["chartSettings"] = item["chart_settings"]
        elif "query" in item:
            body["query"] = item["query"]
        if current:
            saved = api("insight-update", {"id": current["short_id"], **body})
        else:
            saved = api("insight-create", {**body, "dashboards": item["dashboards"]})
        verified = api("insight-get", {"id": saved["short_id"]})
        for key, value in body.items():
            actual = verified.get(key)
            if key == "query":
                actual, value = without_nulls(actual), without_nulls(value)
            if actual != value:
                raise SystemExit(f"read-back mismatch: {item['name']} {key}")
        item["id"] = verified["short_id"]
        # Record each completed mutation so a later failure can resume safely.
        SPEC.write_text(json.dumps(spec, indent=2) + "\n")
        print(f"verified: {item['name']} ({item['id']})")
    for dashboard in spec["dashboards"]:
        api("dashboard-update", dashboard)
        verified = api("dashboard-get", {"id": dashboard["id"]})
        if verified["description"] != dashboard["description"]:
            raise SystemExit("dashboard description read-back mismatch")
        attached = {t["insight"]["short_id"] for t in verified["tiles"] if t.get("insight")}
        expected = {i["id"] for i in spec["insights"] if dashboard["id"] in i.get("dashboards", [])}
        if not expected.issubset(attached):
            raise SystemExit("dashboard is missing expected insights")
        print(f"verified dashboard: {dashboard['id']}")
    for layout in spec.get("dashboard_layouts", []):
        identifiers = {item["name"]: item["id"] for item in spec["insights"] if item.get("id")}
        arrange_dashboard({**layout, "insights": [
            identifiers.get(value, value) for value in layout["insights"]
        ]})


if __name__ == "__main__":
    main()
