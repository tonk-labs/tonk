// The bare /join page's paste field (gooey in-browser/PAGES.md): the step
// starts with focus in its field, except on touch, and a field error goes
// as soon as the field is right.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
const methodSource = (method) => {
  const section = library.split('element!: &join-paste\n')[1].split(/\n(?=[a-z][a-z/-]*!:)/)[0];
  const match = section.match(new RegExp(`\\n    ${method}: \\|\\n((?:      .*\\n|[ \\t]*\\n)+)`));
  assert.ok(match, `join-paste has a ${method} method`);
  return match[1];
};

function page({ path = '/join', coarse = false } = {}) {
  const listeners = {};
  const attrs = new Map([['aria-describedby', 'join-paste-hint']]);
  const focused = [];
  const input = {
    value: '',
    setAttribute: (key, value) => attrs.set(key, value),
    removeAttribute: (key) => attrs.delete(key),
    focus: () => focused.push('input'),
  };
  const error = { textContent: '', hidden: true };
  const form = {
    querySelector: (selector) => selector === 'input[name="link"]' ? input
      : selector === '[data-join-paste-error]' ? error : null,
    closest: () => form,
  };
  input.closest = () => form;
  const window = {
    matchMedia: () => ({ matches: coarse }),
    location: { origin: 'https://tonk.example' },
  };
  const self = {
    contains: () => true,
    closest: (selector) => selector === '[data-path]' ? { dataset: { path } } : null,
    querySelector: (selector) => selector === 'input[name="link"]' ? input : null,
    addEventListener: (type, run) => { listeners[type] = run; },
    dispatchEvent: () => true,
  };
  for (const name of ['link', 'settle', 'join', 'connected']) {
    const method = Function('window', 'CustomEvent', `return (${methodSource(name)})`)(window, class {});
    self[name] = (...args) => method(self, ...args);
  }
  return { self, input, error, form, attrs, focused, listeners };
}

test('the paste field takes focus when /join opens', () => {
  const { self, focused } = page();
  self.connected();
  assert.deepEqual(focused, ['input']);
});

test('the paste field leaves focus alone on touch and on a carried link', () => {
  for (const options of [{ coarse: true }, { path: '/join/https://elsewhere.example/space/x' }]) {
    const { self, focused } = page(options);
    self.connected();
    assert.deepEqual(focused, [], JSON.stringify(options));
  }
});

test('the paste error goes as soon as the field holds a link', () => {
  const { self, input, error, form, attrs, listeners } = page({ path: '/elsewhere' });
  self.connected();
  input.value = 'not a link';
  self.join(form);
  assert.equal(error.hidden, false);
  assert.equal(attrs.get('aria-invalid'), 'true');

  input.value = 'tonk.example/space';
  listeners.input({ target: input });
  assert.equal(error.hidden, false, 'still wrong: an address without its scheme is not a link');

  input.value = 'https://tonk.example/space/did:key:z6Mk#seed';
  listeners.input({ target: input });
  assert.equal(error.hidden, true);
  assert.equal(attrs.has('aria-invalid'), false);
  assert.equal(attrs.get('aria-describedby'), 'join-paste-hint');
});
