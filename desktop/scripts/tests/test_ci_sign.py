"""Exercise signing orchestration and cleanup with fake platform commands."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).parents[1]
CERT = b'fixture certificate DER'
IDENTITY = hashlib.sha1(CERT).hexdigest().upper()
MOCK = '''#!/usr/bin/env python3
import json, os, pathlib, plistlib, sys
root = pathlib.Path(os.environ['SIGN_TEST_ROOT'])
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with (root/'calls').open('a') as log: log.write(name + ':' + (args[0] if args else '') + '\\n')
if name == 'security':
    if args[0] == 'list-keychains' and '-s' not in args: print('"/original/login.keychain-db"')
    elif args[0] == 'list-keychains': (root/'search-list').write_text(json.dumps(args[args.index('-s')+1:]))
    elif args[0] == 'find-identity': print('  1) ' + os.environ['SIGN_TEST_ID'] + ' "Developer ID Application: fixture"')
    elif args[0] == 'cms': sys.stdout.buffer.write((root/'profile.plist').read_bytes())
    elif args[0] == 'import' and os.environ.get('SIGN_TEST_FAILURE') == 'import': sys.exit(1)
elif name == 'nix':
    if os.environ.get('SIGN_TEST_FAILURE') == 'build': sys.exit(1)
    nightly = os.environ.get('TONK_DESKTOP_CHANNEL') == 'nightly'
    app = root/('desktop/.build/Tonk Nightly.app' if nightly else 'desktop/.build/Tonk.app')/'Contents'
    app.mkdir(parents=True)
    info = {'CFBundleIdentifier': 'xyz.tonk.nightly' if nightly else 'xyz.tonk',
            'TonkChannel': 'nightly' if nightly else 'stable'}
    if 'TONK_PROVISIONING_PROFILE' in os.environ:
        info['TonkDirectPasskeyEnabled'] = True
        (app/'embedded.provisionprofile').write_bytes(pathlib.Path(os.environ['TONK_PROVISIONING_PROFILE']).read_bytes())
    (app/'Info.plist').write_bytes(plistlib.dumps(info))
elif name == 'codesign':
    if '-dv' in args: print('TeamIdentifier=8WVKS2F24C', file=sys.stderr)
    if '--entitlements' in args:
        expected = plistlib.loads((root/'profile.plist').read_bytes())['Entitlements']
        if os.environ.get('SIGN_TEST_FAILURE') == 'entitlements': expected['com.apple.developer.associated-domains'] = []
        sys.stdout.buffer.write(plistlib.dumps(expected))
    for arg in args:
        if arg.startswith('--extract-certificates='):
            pathlib.Path(arg.split('=',1)[1]+'0').write_bytes(b'fixture certificate DER')
elif name == 'ditto': pathlib.Path(args[-1]).write_bytes(b'fixture archive')
'''


class SigningLifecycleTests(unittest.TestCase):
    def exercise(self, failure=None, channel=None, nightly_profile=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root/'desktop/scripts'
            scripts.mkdir(parents=True)
            for name in ['ci-sign.sh', 'channel.sh', 'passkey-entitlements.py']:
                shutil.copy(SCRIPTS/name, scripts/name)
            profile = {
                'ExpirationDate': datetime.datetime(2044, 1, 1),
                'DeveloperCertificates': [CERT if failure != 'certificate' else b'other'],
                'Entitlements': {
                    'com.apple.application-identifier': '8WVKS2F24C.xyz.tonk' + ('.nightly' if nightly_profile else ''),
                    'com.apple.developer.team-identifier': '8WVKS2F24C',
                    'com.apple.developer.associated-domains': [
                        'webcredentials:tonk.foundation' if nightly_profile else 'webcredentials:tonk.network'],
                },
            }
            (root/'profile.plist').write_bytes(plistlib.dumps(profile))
            binary = root/'bin'
            binary.mkdir()
            for name in ['security', 'nix', 'codesign', 'ditto']:
                command = binary/name
                command.write_text(MOCK)
                command.chmod(0o755)
            temp = root/'runner'
            temp.mkdir()
            environment = {**os.environ, 'PATH': str(binary)+os.pathsep+os.environ['PATH'],
                           'RUNNER_OS': 'macOS', 'RUNNER_TEMP': str(temp),
                           'MACOS_CERTIFICATE_P12': 'Zml4dHVyZQ==', 'MACOS_CERTIFICATE_PASSWORD': 'fixture-password',
                           'TONK_PROVISIONING_PROFILE_BASE64': 'cHJvZmlsZQ==',
                           'SIGN_TEST_ROOT': str(root), 'SIGN_TEST_ID': IDENTITY,
                           'SIGN_TEST_FAILURE': failure or ''}
            if channel:
                environment['TONK_DESKTOP_CHANNEL'] = channel
                del environment['TONK_PROVISIONING_PROFILE_BASE64']
                if nightly_profile:
                    environment['TONK_NIGHTLY_PROVISIONING_PROFILE_BASE64'] = 'cHJvZmlsZQ=='
            result = subprocess.run(['bash', str(scripts/'ci-sign.sh')], env=environment,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode == 0, failure is None, result.stderr)
            self.assertEqual(list(temp.iterdir()), [], 'Temporary credential directory survives')
            self.assertEqual(json.loads((root/'search-list').read_text()), ['/original/login.keychain-db'])
            self.assertIn('security:delete-keychain', (root/'calls').read_text())
            self.assertNotIn('fixture-password', result.stdout+result.stderr)
            archive = root/'desktop/artifacts/Tonk-macos-arm64.zip'
            self.assertEqual(archive.exists(), failure is None)
            if failure is None:
                profiled = channel is None or nightly_profile
                self.assertEqual('security:cms' in (root/'calls').read_text(), profiled)
            if failure == 'certificate':
                self.assertNotIn('nix:', (root/'calls').read_text())

    def test_success_and_cleanup(self):
        self.exercise()

    def test_nightly_signs_without_passkeys_until_its_profile_exists(self):
        self.exercise(channel='nightly')

    def test_nightly_signs_with_its_own_passkey_profile(self):
        self.exercise(channel='nightly', nightly_profile=True)

    def test_failure_cleanup_and_no_uploadable_archive(self):
        for failure in ['import', 'certificate', 'build', 'entitlements']:
            with self.subTest(failure=failure):
                self.exercise(failure)
