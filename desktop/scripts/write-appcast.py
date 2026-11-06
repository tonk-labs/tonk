"""Write a one-item Sparkle feed for the signed app archive.

Usage: write-appcast.py INFO_PLIST ARCHIVE_URL ENCLOSURE_ATTRIBUTES OUTPUT
ENCLOSURE_ATTRIBUTES is sign_update's output: sparkle:edSignature="..." length="...".
"""
from email.utils import formatdate
import plistlib
import re
import sys
from pathlib import Path
from xml.etree import ElementTree as ET

SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'
ET.register_namespace('sparkle', SPARKLE)

info_path, url, enclosure_attributes, output = sys.argv[1:]
attributes = dict(re.findall(r'([\w:]+)="([^"]*)"', enclosure_attributes))
signature, length = attributes.get('sparkle:edSignature'), attributes.get('length')
if not signature or not (length or '').isdigit():
    raise SystemExit(f'Unexpected sign_update output: {enclosure_attributes}')
info = plistlib.loads(Path(info_path).read_bytes())
version = info['TonkReleaseVersion']

rss = ET.Element('rss', version='2.0')
channel = ET.SubElement(rss, 'channel')
ET.SubElement(channel, 'title').text = info['CFBundleName']
item = ET.SubElement(channel, 'item')
ET.SubElement(item, 'title').text = version
ET.SubElement(item, 'pubDate').text = formatdate(usegmt=True)
ET.SubElement(item, 'description').text = f"Built from commit {info['TonkReleaseCommit']}."
# Sparkle compares CFBundleVersion, which CI stamps with its increasing run number.
ET.SubElement(item, f'{{{SPARKLE}}}version').text = info['CFBundleVersion']
ET.SubElement(item, f'{{{SPARKLE}}}shortVersionString').text = version
ET.SubElement(item, f'{{{SPARKLE}}}minimumSystemVersion').text = info['LSMinimumSystemVersion']
ET.SubElement(item, 'enclosure', {
    'url': url, 'type': 'application/octet-stream', 'length': length,
    f'{{{SPARKLE}}}edSignature': signature,
})
ET.ElementTree(rss).write(output, encoding='utf-8', xml_declaration=True)
