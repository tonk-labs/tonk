#!/usr/bin/env python3
"""Stamp an explicitly selected candidate version without creating a tag."""
import argparse
from pathlib import Path
import re

from release_gate import require


def stamp(root, version):
    require(re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-rc\.[1-9]\d*)?", version),
            "expected X.Y.Z or X.Y.Z-rc.N")
    path = Path(root) / "Cargo.toml"
    manifest = path.read_text()
    section = re.search(r"^\[workspace\.package\]\n(?:(?!^\[).*\n)*", manifest, re.M)
    require(section is not None, "workspace.package missing")
    current = re.search(r'^version = "([^"]+)"', section.group(), re.M)
    require(current is not None, "workspace version missing")
    old = current.group(1)
    packages = (Path(root) / "Cargo.lock").read_text().split("\n[[package]]\n")
    count = 0
    for i, package in enumerate(packages):
        if "\nsource = " not in package and f'\nversion = "{old}"' in package:
            packages[i] = package.replace(f'\nversion = "{old}"', f'\nversion = "{version}"', 1)
            count += 1
    require(count > 0, "no matching workspace lock entries")
    start, end = section.start() + current.start(), section.start() + current.end()
    path.write_text(manifest[:start] + f'version = "{version}"' + manifest[end:])
    (Path(root) / "Cargo.lock").write_text("\n[[package]]\n".join(packages))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--root", default=".")
    args = parser.parse_args()
    stamp(args.root, args.version)
