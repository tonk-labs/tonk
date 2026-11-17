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

// The element runtime refuses a method whose camelCased name an HTMLElement
// already has, so a call to it reaches the DOM's own member instead: a
// `remove` method left `self.remove(...)` detaching the element. Node has no
// DOM, so these are the members a method name is likely to reach for.
const DOM_MEMBERS = new Set([
  'after', 'animate', 'append', 'attachShadow', 'before', 'blur', 'click', 'closest',
  'contains', 'dir', 'focus', 'hidden', 'id', 'inert', 'lang', 'matches', 'normalize',
  'prepend', 'remove', 'replaceChildren', 'replaceWith', 'scroll', 'slot', 'style',
  'title', 'toggleAttribute', 'translate',
]);

test('no element method in the profile library shadows a DOM member', () => {
  const camel = (key) => key.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
  const shadowing = [];
  for (const [, name, body] of library.matchAll(/^element!: &([a-z-]+)\n([\s\S]*?)(?=\n[a-z][a-z/-]*!:|$(?![\s\S]))/gm)) {
    const methods = body.split(/\n  method:\n/)[1]?.split(/\n  [a-z]/)[0] ?? '';
    for (const [, key] of methods.matchAll(/\n    ([a-z-]+): /g)) {
      if (DOM_MEMBERS.has(camel(key))) shadowing.push(`${name}.${key}`);
    }
  }
  assert.deepEqual(shadowing, []);
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
