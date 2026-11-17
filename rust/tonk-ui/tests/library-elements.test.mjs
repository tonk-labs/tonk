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
