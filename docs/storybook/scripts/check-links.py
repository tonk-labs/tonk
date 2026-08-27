#!/usr/bin/env python3
"""Check local links and static asset references in the Storybook."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import unquote


MARKDOWN_LINK = re.compile(r"(?<!!)\[[^]]*]\(([^)]+)\)")
HTML_REFERENCE = re.compile(r"(?:href|src)=[\"']([^\"']+)[\"']")
REMOTE_PREFIXES = ("http://", "https://", "mailto:", "tel:", "data:", "javascript:")


def referenced_paths(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".md":
        return MARKDOWN_LINK.findall(text)
    if path.suffix == ".html":
        return HTML_REFERENCE.findall(text)
    return []


def local_target(raw: str) -> str | None:
    target = raw.strip().strip("<>").split("#", 1)[0]
    if not target or target.startswith(("#", "/")) or target.startswith(REMOTE_PREFIXES):
        return None
    return unquote(target)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path, help="Storybook directory to check")
    args = parser.parse_args()
    root = args.root.resolve()
    errors: list[str] = []
    checked = 0

    for path in sorted(root.rglob("*")):
        if path.suffix not in {".md", ".html"} or not path.is_file():
            continue
        for raw in referenced_paths(path):
            target = local_target(raw)
            if target is None:
                continue
            checked += 1
            resolved = (path.parent / target).resolve()
            if not resolved.exists():
                errors.append(f"{path.relative_to(root)}: missing {raw}")

    if errors:
        print("storybook links:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"storybook links: {checked} local references are valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
