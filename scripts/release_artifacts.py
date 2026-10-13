#!/usr/bin/env python3
"""Content identities for candidate binaries and complete web artifact trees."""
import argparse
import hashlib
from pathlib import Path

from release_gate import require


def file_digest(path):
    with Path(path).open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def tree_digest(path):
    root = Path(path)
    require(root.is_dir(), "artifact tree missing")
    digest = hashlib.sha256()
    files = sorted(root.rglob("*"))
    count = 0
    for file in files:
        require(not file.is_symlink(), "artifact tree contains a symlink")
        if file.is_dir():
            continue
        require(file.is_file(), "artifact tree contains a special file")
        digest.update(file.relative_to(root).as_posix().encode() + b"\0")
        digest.update(file_digest(file).encode() + b"\0")
        count += 1
    require(count > 0, "empty artifact tree")
    return digest.hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path")
    parser.add_argument("--tree", action="store_true")
    parser.add_argument("--expected")
    args = parser.parse_args()
    actual = tree_digest(args.path) if args.tree else file_digest(args.path)
    require(args.expected is None or actual == args.expected, "artifact digest mismatch")
    print(actual)
