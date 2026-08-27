#!/usr/bin/env python3
"""Build and validate the dependency-free visual Storybook data."""

from __future__ import annotations

import argparse
import html
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
BOOK = ROOT / "docs" / "storybook"
APP = BOOK / "app"
JOURNEYS = BOOK / "journey-catalog.md"
SCREENS = BOOK / "screens.json"
BUGS = BOOK / "bug-triage.md"
README = BOOK / "README.md"
VERIFICATION = BOOK / "verification"
OUTPUT_JSON = APP / "data.json"
OUTPUT_JS = APP / "data.js"


def split_row(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def plain(value: str) -> str:
    value = value.replace("<br>", " ").replace("<br/>", " ")
    value = re.sub(r"\[([^]]+)]\([^)]+\)", r"\1", value)
    value = value.replace("`", "")
    return html.unescape(re.sub(r"\s+", " ", value)).strip()


def parse_journeys() -> list[dict[str, str]]:
    group = ""
    journeys: list[dict[str, str]] = []
    for line in JOURNEYS.read_text().splitlines():
        if line.startswith("## ") and line != "## Coverage conclusion":
            group = line.removeprefix("## ").strip()
        if not re.match(r"^\|\s*`[A-Z]+-(?:[A-Z])?\d+`\s*\|", line):
            continue
        cells = split_row(line)
        if len(cells) != 5:
            raise ValueError(f"{JOURNEYS}: malformed journey row: {line}")
        journeys.append(
            {
                "id": plain(cells[0]),
                "group": group,
                "title": plain(cells[1]),
                "variants": plain(cells[2]),
                "evidence": plain(cells[3]),
                "gaps": plain(cells[4]),
            }
        )
    return journeys


def parse_verification() -> list[dict[str, str]]:
    items: list[dict[str, str]] = []
    pattern = re.compile(r"^\|\s*`([A-Z]+-\d+)`\s*\|")
    for path in sorted(VERIFICATION.glob("*.md")):
        if path.name == "README.md":
            continue
        for line in path.read_text().splitlines():
            if not pattern.match(line):
                continue
            cells = split_row(line)
            if len(cells) < 8:
                raise ValueError(f"{path}: malformed verification row: {line}")
            items.append(
                {
                    "id": plain(cells[0]),
                    "priority": plain(cells[1]),
                    "device": plain(cells[2]),
                    "claim": plain(cells[3]),
                    "result": plain(cells[-1]),
                    "file": str(path.relative_to(BOOK)),
                }
            )
    return items


def parse_bugs() -> list[dict[str, str]]:
    bugs: list[dict[str, str]] = []
    for line in BUGS.read_text().splitlines():
        if not re.match(r"^\|\s*`?B-\d+`?\s*\|", line):
            continue
        cells = split_row(line)
        if len(cells) < 5:
            raise ValueError(f"{BUGS}: malformed triage row: {line}")
        bugs.append(
            {
                "id": plain(cells[0]),
                "title": plain(cells[1]),
                "severity": plain(cells[2]).lower(),
                "area": plain(cells[3]),
                "decision": plain(cells[4]),
            }
        )
    return bugs


def parse_coverage() -> list[dict[str, str]]:
    coverage: list[dict[str, str]] = []
    in_coverage = False
    for line in README.read_text().splitlines():
        if line == "## Coverage":
            in_coverage = True
            continue
        if in_coverage and line.startswith("## "):
            break
        if not in_coverage or not re.match(r"^\|\s*`[^`]+`\s*\|", line):
            continue
        cells = split_row(line)
        if len(cells) == 2:
            coverage.append({"document": plain(cells[0]), "status": plain(cells[1])})
    return coverage


def load_screens(journey_ids: set[str]) -> tuple[dict, list[dict]]:
    manifest = json.loads(SCREENS.read_text())
    screens = manifest.get("screens", [])
    seen: set[str] = set()
    mapped: set[str] = set()
    errors: list[str] = []
    for screen in screens:
        screen_id = screen.get("id", "")
        if not re.fullmatch(r"(?:WEB|CLI)-\d{2}", screen_id):
            errors.append(f"invalid screen ID: {screen_id!r}")
        if screen_id in seen:
            errors.append(f"duplicate screen ID: {screen_id}")
        seen.add(screen_id)
        artifact = BOOK / screen.get("artifact", "")
        if not artifact.is_file():
            errors.append(f"{screen_id}: missing artifact {artifact.relative_to(ROOT)}")
        for source in screen.get("source_paths", []):
            if not (ROOT / source).exists():
                errors.append(f"{screen_id}: missing source path {source}")
        for journey_id in screen.get("journey_ids", []):
            if journey_id not in journey_ids:
                errors.append(f"{screen_id}: unknown journey ID {journey_id}")
            mapped.add(journey_id)
    unmapped = sorted(journey_ids - mapped)
    if unmapped:
        errors.append("journeys without a screen: " + ", ".join(unmapped))
    if errors:
        raise ValueError("\n".join(errors))
    return manifest, screens


def duplicates(values: list[str]) -> list[str]:
    return sorted({value for value in values if values.count(value) > 1})


def build() -> dict:
    journeys = parse_journeys()
    journey_ids = [journey["id"] for journey in journeys]
    duplicate_journeys = duplicates(journey_ids)
    if duplicate_journeys:
        raise ValueError("duplicate journey IDs: " + ", ".join(duplicate_journeys))
    verification = parse_verification()
    verification_ids = [item["id"] for item in verification]
    duplicate_verification = duplicates(verification_ids)
    if duplicate_verification:
        raise ValueError(
            "duplicate verification IDs: " + ", ".join(duplicate_verification)
        )
    manifest, screens = load_screens(set(journey_ids))
    bugs = parse_bugs()
    coverage = parse_coverage()
    results = {"unrun": 0, "pass": 0, "fail": 0, "blocked": 0, "other": 0}
    for item in verification:
        result = item["result"].lower()
        if result in {"—", "-", ""}:
            results["unrun"] += 1
        elif result.startswith("pass"):
            results["pass"] += 1
        elif result.startswith("fail"):
            results["fail"] += 1
        elif result.startswith("blocked"):
            results["blocked"] += 1
        else:
            results["other"] += 1
    return {
        "schemaVersion": manifest["schema_version"],
        "auditCommit": manifest["audit_commit"],
        "visualCommit": manifest["visual_commit"],
        "screens": screens,
        "journeys": journeys,
        "verification": verification,
        "verificationResults": results,
        "bugs": bugs,
        "coverage": coverage,
    }


def rendered(data: dict) -> tuple[str, str]:
    payload = json.dumps(data, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    return payload, "window.STORYBOOK_DATA = " + payload.rstrip() + ";\n"


def check_impact(base: str, manifest: dict) -> None:
    command = ["git", "diff", "--name-only", f"{base}...HEAD"]
    changed = subprocess.run(
        command, cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout.splitlines()
    tracked = tuple(manifest["tracked_product_paths"])
    product_changed = [path for path in changed if path.startswith(tracked)]
    storybook_changed = [path for path in changed if path.startswith("docs/storybook/")]
    if product_changed and not storybook_changed:
        raise ValueError(
            "user-facing product paths changed without a docs/storybook update:\n- "
            + "\n- ".join(product_changed)
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if output is stale")
    parser.add_argument("--base", help="also enforce Storybook impact since this git base")
    args = parser.parse_args()
    try:
        data = build()
        manifest = json.loads(SCREENS.read_text())
        if args.base:
            check_impact(args.base, manifest)
        output_json, output_js = rendered(data)
        if args.check:
            stale = []
            if not OUTPUT_JSON.is_file() or OUTPUT_JSON.read_text() != output_json:
                stale.append(str(OUTPUT_JSON.relative_to(ROOT)))
            if not OUTPUT_JS.is_file() or OUTPUT_JS.read_text() != output_js:
                stale.append(str(OUTPUT_JS.relative_to(ROOT)))
            if stale:
                raise ValueError(
                    "generated Storybook data is stale; run "
                    "python3 docs/storybook/scripts/build.py:\n- " + "\n- ".join(stale)
                )
        else:
            APP.mkdir(parents=True, exist_ok=True)
            OUTPUT_JSON.write_text(output_json)
            OUTPUT_JS.write_text(output_js)
    except (ValueError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"storybook: {error}", file=sys.stderr)
        return 1
    print(
        "storybook: "
        f"{len(data['screens'])} screens, "
        f"{len(data['journeys'])} journeys, "
        f"{len(data['verification'])} verification items, "
        f"{len(data['bugs'])} triage findings"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
