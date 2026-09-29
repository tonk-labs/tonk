import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
const section = library.split('element!: &hub-discover\n')[1];
const method = name => Function(`return (${section.split(`    ${name}: |\n`)[1].split(/\n    \S|\n\S/)[0].trim()})`)();
const catalog = JSON.parse(readFileSync(new URL('../../tonk-worker/tests/fixtures/discover/catalog.json', import.meta.url)));

test('remote metadata is escaped and catalog references are resolved without bundled assets', () => {
  const template = structuredClone(catalog.templates[0]);
  template.name = '<script>bad()</script>';
  const card = method('card')({}, template, 'https://example.com/catalog.json');
  assert.ok(card.includes('&lt;script&gt;'));
  assert.ok(!card.includes('<script>'));
  assert.ok(card.includes('https://example.com/preview.svg'));
  assert.ok(card.includes('https://example.com/catalog.json#remote-demo'));
  template.images[0].url = 'javascript:alert(1)';
  assert.throws(() => method('card')({}, template, 'https://example.com/catalog.json'));
});

test('catalog failure can retry, optional files are not fetched while browsing, and tab revisits reuse cards', async () => {
  const nodes = { '[data-discover-status]': {}, '[data-discover-retry]': {}, '.template-grid': {} };
  const self = { isConnected: true, getAttribute: () => 'https://example.com/catalog.json',
    querySelector: key => nodes[key], card: template => method('card')(self, template, 'https://example.com/catalog.json') };
  let calls = 0;
  const previous = globalThis.fetch;
  try {
    globalThis.fetch = async url => { calls++; assert.equal(url, 'https://example.com/catalog.json'); throw new Error('offline'); };
    await method('load')(self);
    assert.equal(nodes['[data-discover-retry]'].hidden, false);
    assert.match(nodes['[data-discover-status]'].textContent, /Couldn’t load/);
    globalThis.fetch = async () => { calls++; return { ok: true, json: async () => catalog }; };
    await method('load')(self);
    assert.equal(nodes['[data-discover-retry]'].hidden, true);
    assert.match(nodes['.template-grid'].innerHTML, /Remote demo/);
    await method('load')(self);
    assert.equal(calls, 2);
  } finally { globalThis.fetch = previous; }
});
