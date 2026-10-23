"""Exercise the notarization boundary without Apple credentials or services."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / 'ci-notarize.sh'
MOCK = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
failure = os.environ.get('FAILURE', '')
root = pathlib.Path(os.environ['TEST_ROOT'])
with (root/'calls').open('a') as f: f.write(name + ' ' + ' '.join(args[:2]) + '\\n')
if name == 'xcrun':
    if args[:2] == ['notarytool', 'submit']:
        if failure == 'submit': sys.exit(1)
        print(json.dumps({'id': 'fixture', 'status': 'Invalid' if failure == 'reject' else 'Accepted'}))
    elif failure == 'staple' and args[:2] == ['stapler', 'staple']: sys.exit(1)
    elif failure == 'validate' and args[:2] == ['stapler', 'validate']: sys.exit(1)
elif name == 'spctl' and failure == 'assess': sys.exit(1)
elif name == 'codesign' and failure == 'signature': sys.exit(1)
elif name == 'ditto': pathlib.Path(args[-1]).write_bytes(b'notarized fixture')
'''


class NotarizationTests(unittest.TestCase):
    def exercise(self, failure=''):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root/'desktop/scripts'
            scripts.mkdir(parents=True)
            shutil.copy(SCRIPT, scripts/SCRIPT.name)
            shutil.copy(SCRIPT.parent/'channel.sh', scripts/'channel.sh')
            artifacts = root/'desktop/artifacts'
            artifacts.mkdir()
            archive = artifacts/'Tonk-macos-arm64.zip'
            checksum = artifacts/'Tonk-macos-arm64.sha256'
            archive.write_bytes(b'old unsigned archive')
            checksum.write_text('stale checksum')
            binary = root/'bin'
            binary.mkdir()
            runner = root/'runner'
            runner.mkdir()
            for name in ['xcrun', 'codesign', 'spctl', 'ditto']:
                command = binary/name
                command.write_text(MOCK)
                command.chmod(0o755)
            env = dict(os.environ, PATH=str(binary)+os.pathsep+os.environ['PATH'],
                       TEST_ROOT=str(root), RUNNER_TEMP=str(runner), FAILURE=failure,
                       APP_STORE_CONNECT_KEY_P8='secret fixture',
                       APP_STORE_CONNECT_KEY_ID='fixture', APP_STORE_CONNECT_ISSUER_ID='fixture')
            result = subprocess.run(['bash', str(scripts/SCRIPT.name)], env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode == 0, not failure, result.stderr)
            self.assertEqual(archive.exists(), not failure)
            self.assertEqual(checksum.exists(), not failure)
            self.assertEqual(list(runner.iterdir()), [])
            self.assertNotIn('secret fixture', result.stdout + result.stderr)
            if not failure:
                calls = (root/'calls').read_text()
                self.assertLess(calls.index('xcrun stapler validate'), calls.rindex('ditto'))
                self.assertIn('Tonk-macos-arm64.zip', checksum.read_text())

    def test_accepted_and_stapled(self):
        self.exercise()

    def test_fail_closed(self):
        for failure in ['submit', 'reject', 'staple', 'validate', 'signature', 'assess']:
            with self.subTest(failure=failure):
                self.exercise(failure)
