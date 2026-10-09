import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
const section = library.split('element!: &hub-discover\n')[1];
const method = name => Function(`return (${section.split(`    ${name}: |\n`)[1].split(/\n    \S|\n\S/)[0].trim()})`)();
const catalog = JSON.parse(readFileSync(new URL('../../tonk-worker/tests/fixtures/discover/catalog.json', import.meta.url)));

// A stand-in for the element: its methods called the way the element
// runtime calls them (self first), over a fixed set of DOM nodes.
function element({ attribute = 'https://example.com/catalog.json', rows = [] } = {}) {
  const nodes = {
    '[data-discover-status]': {}, '[data-discover-retry]': {}, '[data-discover-errors]': {},
    '.template-grid': {}, '[data-discover-default-url]': {},
  };
  const self = {
    isConnected: true, nodes,
    getAttribute: () => attribute,
    querySelector: key => nodes[key],
    querySelectorAll: () => rows.map(url => ({ dataset: { url } })),
  };
  for (const name of ['escape', 'label', 'card', 'deployment', 'catalogs', 'load']) {
    self[name] = (...args) => method(name)(self, ...args);
  }
  self.fetchCatalog = (...args) => method('fetch-catalog')(self, ...args);
  return self;
}

async function withFetch(impl, body) {
  const previous = globalThis.fetch;
  globalThis.fetch = impl;
  try { return await body(); } finally { globalThis.fetch = previous; }
}

const ok = body => ({ ok: true, json: async () => body });

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

test('cards name their catalog and prefix every id with its position', () => {
  const template = catalog.templates[0];
  const first = method('card')({}, template, 'https://a.example/catalog.json', 'Honky <Tonks>', 0);
  const second = method('card')({}, template, 'https://b.example/catalog.json', 'b.example', 1);
  assert.ok(first.includes('from Honky &lt;Tonks&gt;'));
  assert.ok(!first.includes('Honky Tonks.'), 'no hard-coded source');
  assert.ok(first.includes('id="copy-template-0-remote-demo"') && first.includes('form="copy-template-0-remote-demo"'));
  assert.ok(second.includes('id="copy-template-1-remote-demo"') && second.includes('form="copy-template-1-remote-demo"'));
  // The selector the end-to-end suite drives cards by stays the slug.
  assert.ok(first.includes('data-template="remote-demo"'));
  assert.equal(method('label')({}, { name: ' Honky Tonks ' }, 'https://a.example/c.json'), 'Honky Tonks');
  assert.equal(method('label')({}, {}, 'https://b.example:8443/c.json'), 'b.example:8443');
});

test('catalog failure can retry, optional files are not fetched while browsing, and tab revisits reuse cards', async () => {
  const self = element();
  let calls = 0;
  await withFetch(async url => {
    calls++;
    assert.equal(url, 'https://example.com/catalog.json');
    throw new Error('offline');
  }, () => self.load());
  assert.equal(self.nodes['[data-discover-retry]'].hidden, false);
  assert.match(self.nodes['[data-discover-status]'].textContent, /Couldn’t load/);
  await withFetch(async url => {
    calls++;
    assert.equal(url, 'https://example.com/catalog.json');
    return ok(catalog);
  }, async () => {
    await self.load();
    assert.equal(self.nodes['[data-discover-retry]'].hidden, true);
    assert.match(self.nodes['.template-grid'].innerHTML, /Remote demo/);
    await self.load();
  });
  assert.equal(calls, 2);
});

test('the deployment default wins over the attribute, and the owner’s catalogs list after it once each', async () => {
  const deployment = 'https://deploy.example/catalog.json';
  const own = 'http://localhost:8777/catalog.json';
  const self = element({ rows: [own, deployment, 'not a url'] });
  // The endpoint is fetched host-relative, through the guest's relay.
  globalThis.window = { tonk: { ready: Promise.resolve() } };
  const fetched = [];
  try {
    await withFetch(async url => {
      fetched.push(url);
      if (url === '/.well-known/tonk/discover') return ok({ catalog: deployment });
      if (url === deployment) return ok({ ...catalog, name: 'Honky Tonks' });
      if (url === own) return ok(catalog);
      throw new Error(`unexpected ${url}`);
    }, () => self.load());
  } finally { delete globalThis.window; }
  assert.deepEqual(fetched, ['/.well-known/tonk/discover', deployment, own]);
  assert.equal(self.nodes['[data-discover-default-url]'].textContent, deployment);
  const grid = self.nodes['.template-grid'].innerHTML;
  assert.ok(grid.includes('copy-template-0-remote-demo') && grid.includes('copy-template-1-remote-demo'),
    'the same slug from two catalogs renders twice without colliding');
  assert.ok(grid.includes('from Honky Tonks') && grid.includes('from localhost:8777'));
});

test('a null or missing endpoint falls back to the attribute', async () => {
  for (const answer of [ok({ catalog: null }), { ok: false }]) {
    const self = element({ attribute: 'https://fallback.example/catalog.json' });
    const fetched = [];
    globalThis.window = { tonk: { ready: Promise.resolve() } };
    try {
      await withFetch(async url => {
        fetched.push(url);
        return url === '/.well-known/tonk/discover' ? answer : ok(catalog);
      }, () => self.load());
    } finally { delete globalThis.window; }
    assert.deepEqual(fetched, ['/.well-known/tonk/discover', 'https://fallback.example/catalog.json']);
    assert.match(self.nodes['.template-grid'].innerHTML, /fallback\.example\/catalog\.json#remote-demo/);
  }
});

test('one failing catalog says so on its own and the others still list', async () => {
  const self = element({ rows: ['https://broken.example/catalog.json'] });
  await withFetch(async url => {
    if (url === 'https://example.com/catalog.json') return ok(catalog);
    throw new Error('offline');
  }, () => self.load());
  assert.match(self.nodes['.template-grid'].innerHTML, /Remote demo/);
  assert.match(self.nodes['[data-discover-errors]'].innerHTML, /broken\.example/);
  assert.equal(self.nodes['[data-discover-status]'].textContent, '');
  assert.equal(self.nodes['[data-discover-retry]'].hidden, false);
});
