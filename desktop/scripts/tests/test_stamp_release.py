import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / 'stamp-release.py'


class ReleaseIdentityTests(unittest.TestCase):
    def test_channels_and_invalid_version(self):
        for version in ['0.7.0', '0.7.0-rc.1', '0.7.0-dev.abcdef123456', 'bad/version']:
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)/'Info.plist'
                original = plistlib.dumps({'CFBundleIdentifier': 'xyz.tonk'})
                path.write_bytes(original)
                result = subprocess.run(['python3', str(SCRIPT), str(path)],
                                        env=dict(os.environ, TONK_DESKTOP_VERSION=version,
                                                 GITHUB_RUN_NUMBER='123', GITHUB_SHA='abc123'),
                                        capture_output=True, text=True)
                if version == 'bad/version':
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(path.read_bytes(), original)
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    info = plistlib.loads(path.read_bytes())
                    self.assertEqual(info['CFBundleShortVersionString'], '0.7.0')
                    self.assertEqual(info['CFBundleIdentifier'], 'xyz.tonk')
                    self.assertEqual(info['CFBundleVersion'], '123')
                    self.assertEqual(info['TonkReleaseVersion'], version)
                    self.assertEqual(info['TonkReleaseCommit'], 'abc123')
