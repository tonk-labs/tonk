#!/usr/bin/env python3
"""Print local Cargo overrides for an independently patched dialog-db checkout."""

import json
from pathlib import Path
import re
import sys


root = Path(sys.argv[1]).resolve(strict=True)
print('[patch."https://github.com/dialog-db/dialog-db.git"]')
for manifest in sorted((root / "rust").glob("*/Cargo.toml")):
    name = re.search(r'^name\s*=\s*"(dialog-[^"]+)"', manifest.read_text(), re.M)
    if name:
        print(f'{json.dumps(name[1])} = {{ path = {json.dumps(str(manifest.parent))} }}')
