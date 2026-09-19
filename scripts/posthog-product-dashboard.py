#!/usr/bin/env python3
"""Validate and publish the versioned Product decisions dashboard."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = ROOT / "docs/analytics/product-decisions-dashboard.json"


def cli_path() -> str:
    configured = os.environ.get("POSTHOG_CLI")
    found = configured or shutil.which("posthog-cli")
    if not found:
        raise SystemExit("posthog-cli is not installed or POSTHOG_CLI is unset")
    return str(pathlib.Path(found).resolve())


def api(tool: str, payload: dict[str, object]) -> object:
    command = [
        cli_path(),
        "api",
        "call",
        "--json",
        tool,
        json.dumps(payload, separators=(",", ":")),
    ]
    completed = subprocess.run(command, check=False, capture_output=True, text=True)
    if completed.returncode:
        print(completed.stderr, file=sys.stderr, end="")
        raise SystemExit(f"PostHog {tool} failed with exit {completed.returncode}")
    return json.loads(completed.stdout)


def query_node(insight: dict[str, object]) -> dict[str, object]:
    return {
        "kind": "DataVisualizationNode",
        "source": {"kind": "HogQLQuery", "query": insight["query"]},
        "display": insight["display"],
    }


def validate(spec: dict[str, object]) -> None:
    names: set[str] = set()
    for insight in spec["insights"]:
        name = insight["name"]
        if name in names:
            raise SystemExit(f"duplicate insight name: {name}")
        names.add(name)
        result = api("execute-sql", {"query": insight["query"]})
        if isinstance(result, dict) and "error" in result:
            raise SystemExit(f"query failed for {name}: {result['error']}")
        print(f"validated: {name}")


def exact_dashboard(name: str) -> dict[str, object] | None:
    result = api("dashboards-get-all", {"search": name, "limit": 100})
    return next((row for row in result.get("results", []) if row.get("name") == name), None)


def exact_insight(name: str) -> dict[str, object] | None:
    result = api("insights-list", {"search": name, "saved": True, "limit": 100})
    return next((row for row in result.get("results", []) if row.get("name") == name), None)


def publish(spec: dict[str, object]) -> None:
    dashboard_spec = spec["dashboard"]
    dashboard = exact_dashboard(dashboard_spec["name"])
    if dashboard is None:
        dashboard = api("dashboard-create", dashboard_spec)
    else:
        dashboard = api(
            "dashboard-update",
            {
                "id": dashboard["id"],
                "name": dashboard_spec["name"],
                "description": dashboard_spec["description"],
                "pinned": dashboard_spec["pinned"],
                "tags": dashboard_spec["tags"],
            },
        )
    dashboard_id = int(dashboard["id"])
    for related in spec.get("related_dashboards", []):
        updated = api(
            "dashboard-update",
            {"id": related["id"], "description": related["description"]},
        )
        if updated.get("description") != related["description"]:
            raise SystemExit(f"dashboard description read-back mismatch for {related['id']}")
        print(f"updated dashboard description: {related['id']}")
    published_insights: list[dict[str, object]] = []
    for insight in spec["insights"]:
        body = {
            "name": insight["name"],
            "description": insight["description"],
            "dashboards": [dashboard_id],
            "tags": ["product", "analytics-v1", "privacy-reviewed"],
            "query": query_node(insight),
        }
        current = exact_insight(insight["name"])
        if current is None:
            saved = api("insight-create", body)
        else:
            body["id"] = current["short_id"]
            saved = api("insight-update", body)
        verified = api("insight-get", {"id": saved["short_id"]})
        if verified.get("query") != body["query"]:
            raise SystemExit(f"read-back query mismatch for {insight['name']}")
        published_insights.append(
            {
                "name": insight["name"],
                "id": verified["id"],
                "short_id": verified["short_id"],
            }
        )
        print(f"published: {insight['name']} ({verified['short_id']})")
    verified_dashboard = api("dashboard-get", {"id": dashboard_id})
    attached = {
        tile.get("insight", {}).get("short_id")
        for tile in verified_dashboard.get("tiles", [])
        if tile.get("insight")
    }
    expected = {item["short_id"] for item in published_insights}
    if not expected.issubset(attached):
        raise SystemExit("dashboard read-back is missing one or more published insights")
    spec["published"] = {
        "dashboard_id": dashboard_id,
        "verified_at": dt.datetime.now(dt.UTC).isoformat(),
        "insights": published_insights,
    }
    SPEC.write_text(json.dumps(spec, indent=2) + "\n")
    print(f"verified dashboard {dashboard_id}: {len(expected)} insights attached")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate", "publish"))
    args = parser.parse_args()
    spec = json.loads(SPEC.read_text())
    validate(spec)
    if args.mode == "publish":
        publish(spec)


if __name__ == "__main__":
    main()
