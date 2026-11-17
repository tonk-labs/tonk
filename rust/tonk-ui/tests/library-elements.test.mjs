import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

// Every `element!:` method in the profile library is a JavaScript function
// the guest compiles when it defines the element. One that does not parse
// leaves the whole element undefined, and everything inside it inert, with
// nothing said; so each must at least compile.
const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');

test('every element method in the profile library compiles', () => {
  const names = [...library.matchAll(/^element!: &([a-z-]+)$/gm)].map((match) => match[1]);
  assert.ok(names.length > 0, 'the profile library defines elements');
  const broken = [];
  let methods = 0;
  for (const name of names) {
    const section = library.split(`element!: &${name}\n`)[1].split(/\n(?=[a-z][a-z/-]*!:)/)[0];
    for (const [, method, source] of section.matchAll(/\n    ([a-z-]+): \|\n((?:      .*\n|[ \t]*\n)+)/g)) {
      methods++;
      try { Function(`return (${source})`); } catch (error) { broken.push(`${name}.${method}: ${error.message}`); }
    }
  }
  assert.ok(methods > 0, 'the elements have methods');
  assert.deepEqual(broken, []);
});

// The source of one element's method, as the guest compiles it.
const methodSource = (element, method) => {
  const section = library.split(`element!: &${element}\n`)[1].split(/\n(?=[a-z][a-z/-]*!:)/)[0];
  const match = section.match(new RegExp(`\\n    ${method}: \\|\\n((?:      .*\\n|[ \\t]*\\n)+)`));
  assert.ok(match, `${element} has a ${method} method`);
  return match[1];
};

test("the hub menu's copy link copies the space's address at once, waiting on nothing", async () => {
  const written = [];
  const window = {
    tonk: { context: { origin: 'https://tonk.example' } },
    location: { origin: 'null' },
    fetch: () => { throw new Error('copy link must not fetch'); },
  };
  const navigator = { clipboard: { writeText: async (text) => { written.push(text); } } };
  const timers = [];
  const copyLink = Function('window', 'navigator', 'setTimeout', 'clearTimeout',
    `return (${methodSource('space-actions', 'copy-link')})`)(
    window, navigator, (run, ms) => { timers.push(ms); return 1; }, () => {});
  const attrs = new Map();
  const button = {
    dataset: { space: 'did:key:z6Mkspace' },
    setAttribute: (name, value) => attrs.set(name, value),
    removeAttribute: (name) => attrs.delete(name),
  };
  const self = { close: () => {} };
  const result = await copyLink(self, button);
  assert.deepEqual(written, ['https://tonk.example/space/did:key:z6Mkspace']);
  assert.deepEqual(result, { url: 'https://tonk.example/space/did:key:z6Mkspace', copied: true });
  assert.equal(attrs.get('data-copy'), 'copied');
  assert.deepEqual(timers, [900]);

  navigator.clipboard.writeText = async () => { throw new Error('denied'); };
  const refused = await copyLink(self, button);
  assert.equal(refused.copied, false);
  assert.equal(attrs.get('data-copy'), 'failed');
});

test("every hub dialog's heading is lower case, as the hub's sheets are", () => {
  const headings = [...library.matchAll(/<tonk-dialog appearance="hub"[^>]*\bheading="([^"]+)"/g)].map((match) => match[1]);
  assert.ok(headings.length > 0, 'the library has hub dialogs');
  assert.deepEqual(headings.filter((heading) => heading !== heading.toLowerCase()), []);
});

test('the seed page is a hub page, so its dialog wears the hub look', () => {
  const view = library.split('this: tonk:seed/route\n  show:')[1].split('\nroute!:')[0];
  assert.match(view, /class="hub-page tonk-hub seed-page"/);
  assert.match(view, /with:src="look@space"/);
  assert.match(view, /with:src="hub@space"/);
  assert.match(view, /class="hub-sheet"/);
});

test('the space menu steps out of sight under its delete dialog and comes back after it', () => {
  const connected = methodSource('space-remove', 'connected');
  assert.match(connected, /setAttribute\('data-behind', ''\)/);
  assert.match(connected, /'fabb-close'/);
  assert.match(connected, /removeAttribute\('data-behind'\)/);
  assert.doesNotMatch(connected, /menu\.close\?\.\(\)/);
  assert.match(library, /\.space-menu\[data-behind\][^{]*\{[^}]*visibility:hidden/);
  assert.match(library, /\.space-menu\[data-behind\] tonk-dialog \{ visibility:visible; \}/);
});

test("the mobile nav's button rules stop at its own controls, not its dialog's", () => {
  assert.doesNotMatch(library, /\.mobile-nav button\b/);
  assert.match(library, /\.mobile-nav > a, \.mobile-nav \.mobile-create \{ display:flex; width:100%;/);
});
