import datetime
import hashlib
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('passkeys', Path(__file__).parents[1] / 'passkey-entitlements.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime.datetime(2026, 1, 1)
        self.cert = b'test certificate DER fixture'
        self.identity = hashlib.sha1(self.cert).hexdigest()
        self.profile = {
            'ExpirationDate': self.now + datetime.timedelta(days=1),
            'DeveloperCertificates': [self.cert],
            'Entitlements': {
                'com.apple.application-identifier': module.TEAM + '.xyz.tonk',
                'com.apple.developer.team-identifier': module.TEAM,
                'com.apple.developer.associated-domains': [module.DOMAIN],
            },
        }

    def validate(self):
        return module.entitlements_for(self.profile, self.identity, self.now)

    def test_matching_certificate_and_profile(self):
        self.assertEqual(self.validate()['com.apple.developer.associated-domains'], [module.DOMAIN])

    def test_wildcard_is_narrowed(self):
        self.profile['Entitlements']['com.apple.developer.associated-domains'] = ['*']
        self.assertEqual(self.validate()['com.apple.developer.associated-domains'], [module.DOMAIN])

    def test_wrong_app_team_or_domain(self):
        for key in ['com.apple.application-identifier', 'com.apple.developer.team-identifier', 'com.apple.developer.associated-domains']:
            with self.subTest(key=key):
                original = self.profile['Entitlements'][key]
                self.profile['Entitlements'][key] = ['webcredentials:other.example'] if isinstance(original, list) else 'other'
                with self.assertRaises(ValueError):
                    self.validate()
                self.profile['Entitlements'][key] = original

    def test_expired_profile(self):
        self.profile['ExpirationDate'] = self.now
        with self.assertRaises(ValueError):
            self.validate()

    def test_unmatched_certificate(self):
        self.profile['DeveloperCertificates'] = [b'another certificate']
        with self.assertRaises(ValueError):
            self.validate()

    def test_missing_certificate(self):
        del self.profile['DeveloperCertificates']
        with self.assertRaises(ValueError):
            self.validate()
