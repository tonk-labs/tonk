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
