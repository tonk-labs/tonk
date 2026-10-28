from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from xml.etree import ElementTree as ET

SCRIPT = Path(__file__).parents[1] / 'write-appcast.py'
SPARKLE = '{http://www.andymatuschak.org/xml-namespaces/sparkle}'
INFO = {'CFBundleName': 'Tonk Nightly', 'CFBundleVersion': '812', 'LSMinimumSystemVersion': '15.0',
        'TonkReleaseVersion': '0.7.0-dev.abcdef123456', 'TonkReleaseCommit': 'abc123'}
URL = 'https://example.test/releases/download/tonk-desktop-dev/Tonk-macos-arm64.zip'


class AppcastTests(unittest.TestCase):
    def write(self, enclosure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'Info.plist').write_bytes(plistlib.dumps(INFO))
            result = subprocess.run(['python3', str(SCRIPT), str(root/'Info.plist'), URL, enclosure,
                                     str(root/'appcast.xml')], capture_output=True, text=True)
            feed = (root/'appcast.xml').read_text() if result.returncode == 0 else None
            return result, feed

    def test_feed_offers_the_signed_archive_by_build_number(self):
        result, feed = self.write('sparkle:edSignature="c2ln+/==" length="1234"')
        self.assertEqual(result.returncode, 0, result.stderr)
        item = ET.fromstring(feed).find('channel/item')
        self.assertEqual(item.findtext(f'{SPARKLE}version'), '812')
        self.assertEqual(item.findtext(f'{SPARKLE}shortVersionString'), '0.7.0-dev.abcdef123456')
        self.assertEqual(item.findtext(f'{SPARKLE}minimumSystemVersion'), '15.0')
        self.assertIn('abc123', item.findtext('description'))
        enclosure = item.find('enclosure')
        self.assertEqual(enclosure.get('url'), URL)
        self.assertEqual(enclosure.get('length'), '1234')
        self.assertEqual(enclosure.get(f'{SPARKLE}edSignature'), 'c2ln+/==')

    def test_unexpected_signer_output_writes_no_feed(self):
        for enclosure in ['', 'length="1234"', 'sparkle:edSignature="c2ln" length="many"']:
            with self.subTest(enclosure=enclosure):
                result, feed = self.write(enclosure)
                self.assertNotEqual(result.returncode, 0)
                self.assertIsNone(feed)
