"""Stamp release identity before signing; retain a numeric macOS version."""
import os
import plistlib
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
version = os.environ['TONK_DESKTOP_VERSION']
match = re.fullmatch(r'(\d+\.\d+\.\d+)(?:-[0-9A-Za-z.-]+)?', version)
if not match:
    raise SystemExit(f'Invalid desktop version: {version}')
info = plistlib.loads(path.read_bytes())
info.update(CFBundleShortVersionString=match[1],
            CFBundleVersion=os.environ['GITHUB_RUN_NUMBER'],
            TonkReleaseVersion=version,
            TonkReleaseCommit=os.environ['GITHUB_SHA'])
path.write_bytes(plistlib.dumps(info))
