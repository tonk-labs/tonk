// A new passlink for the same space, opened in the same tab, changes only
// the fragment: the page neither reloads nor re-routes, so `<page-mount>`
// mounts again when the host relays the moved location.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
const methodSource = (method) => {
  const section = library.split('element!: &page-mount\n')[1].split(/\n(?=[a-z][a-z/-]*!:)/)[0];
  const match = section.match(new RegExp(`\\n    ${method}: \\|\\n((?:      .*\\n|[ \\t]*\\n)+)`));
  assert.ok(match, `page-mount has a ${method} method`);
  return match[1];
};

function mounted() {
  const listeners = {};
  const window = {
    tonk: { context: { origin: 'https://tonk.example', path: '/space/did:key:zSpace', search: '', hash: '#first' } },
    location: { href: 'about:srcdoc' },
    addEventListener: (type, run) => { listeners[type] = run; },
    removeEventListener: (type) => { delete listeners[type]; },
  };
  class CustomEvent { constructor(type, init) { this.type = type; this.detail = init.detail; } }
  class MutationObserver { observe() {} disconnect() {} }
  const mounts = [];
  const self = {
    closest: () => ({ parentElement: null }),
    addEventListener: () => {},
    dispatchEvent: (event) => { mounts.push(event.detail.hash); return true; },
  };
  for (const name of ['location', 'mount', 'moved', 'ready', 'connected', 'disconnected']) {
    const method = Function('window', 'CustomEvent', 'MutationObserver', `return (${methodSource(name)})`)(
      window, CustomEvent, MutationObserver);
    self[name] = (...args) => method(self, ...args);
  }
  self.connected();
  return { self, window, listeners, mounts };
}

test('a fragment-only move mounts the page again', () => {
  const { window, listeners, mounts } = mounted();
  assert.deepEqual(mounts, ['#first']);
  window.tonk.context = { ...window.tonk.context, hash: '#second' };
  listeners['tonk:context']();
  assert.deepEqual(mounts, ['#first', '#second']);
});

test('the same location or a new route does not mount again', () => {
  const { window, listeners, mounts } = mounted();
  listeners['tonk:context']();
  window.tonk.context = { ...window.tonk.context, path: '/settings', hash: '#other' };
  listeners['tonk:context']();
  assert.deepEqual(mounts, ['#first']);
});

test('a disconnected mount stops listening', () => {
  const { self, listeners } = mounted();
  self.disconnected();
  assert.equal(listeners['tonk:context'], undefined);
});
