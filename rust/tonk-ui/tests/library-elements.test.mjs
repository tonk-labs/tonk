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

test("a space's default passlink stays its oldest open link when this tab makes another", () => {
  const made = new Map([['bafy-new', 'https://tonk.example/space/x#new']]);
  const rows = Function(`return (${methodSource('space-links', 'rows')})`)();
  const self = { made: () => made };
  // The worker lists oldest first; the new link was made in this tab.
  const invites = [
    { kind: 'open', status: 'active', targetCid: 'bafy-old', invitation: 'i1' },
    { kind: 'open', status: 'active', targetCid: 'bafy-new', invitation: 'i2' },
  ];
  const listed = rows(self, 'did:key:space', invites, [], []);
  assert.deepEqual(listed.map((row) => [row.target, !!row.default]), [['bafy-old', true], ['bafy-new', false]]);
});

test('the default passlink offers reset, others remove, and reset carries the column hook', () => {
  const source = methodSource('space-links', 'render');
  assert.match(source, /data-link-ask="reset" data-ask="reset"/);
  assert.match(source, /data-link-ask="remove"/);
});

// Compile a method with its free names bound to stand-ins.
const method = (element, name, scope = {}) =>
  Function(...Object.keys(scope), `return (${methodSource(element, name)})`)(...Object.values(scope));

const linksSelf = (overrides = {}) => {
  const self = {
    dataset: { space: 'did:key:space', name: 'Field notes' },
    _rows: [{ kind: 'open', target: 'bafy-a', default: true }, { kind: 'open', target: 'bafy-b' }],
    made: () => new Map(),
    act: (button, word, run) => run(),
    api: async () => { throw new Error('raw worker detail 500'); },
    render: () => {},
    load: async () => {},
    ...overrides,
  };
  self.linkName = (row) => method('space-links', 'link-name')(self, row);
  return self;
};

test('a link that could not be removed says so in the line, in the mock words, with try again', async () => {
  const failed = [];
  const self = linksSelf({ hub: () => ({ fail: (text, retry, wait) => failed.push({ text, retry, wait }) }), _ask: { what: 'remove' } });
  await method('space-links', 'unlink')(self, null, 'bafy-b', 'open');
  assert.equal(failed.length, 1);
  assert.equal(failed[0].text, 'Couldn’t remove the passlink, so it still works.');
  assert.equal(failed[0].wait, 'removing');
  assert.equal(typeof failed[0].retry, 'function');
  assert.equal(self._ask, null);
});

test('a reset that did not land takes the question, and its answer becomes try again', async () => {
  const self = linksSelf({ hub: () => ({}), _ask: { what: 'reset', target: 'bafy-a' }, mint: async () => { throw new Error('raw'); } });
  await method('space-links', 'reset')(self, null, 'bafy-a');
  assert.equal(self._ask.failed, 'Couldn’t reset the passlink, so the old one still works.');
  self.esc = (text) => method('space-links', 'esc')(self, text);
  const html = method('space-links', 'ask-html')(self, self._rows[0]);
  assert.match(html, /Couldn’t reset the passlink, so the old one still works\./);
  assert.match(html, />try again<\/button>/);
  assert.doesNotMatch(html, /raw/);
});

test('a reset asks the worker to list the new link in the place of the one it replaces', async () => {
  const minted = [];
  const calls = [];
  const self = linksSelf({
    hub: () => ({}),
    _ask: { what: 'reset', target: 'bafy-a' },
    mint: async (replaces) => { minted.push(replaces); return { copied: true }; },
    api: async (path, body) => { calls.push([path, body]); return {}; },
  });
  await method('space-links', 'reset')(self, null, 'bafy-a');
  assert.deepEqual(minted, ['bafy-a']);
  assert.match(calls[0][0], /invites\/bafy-a\/revoke$/);

  const posted = [];
  const mintSelf = {
    dataset: { space: 'did:key:space' },
    made: () => new Map(),
    api: async (path, body) => { posted.push([path, body]); return path.endsWith('/invite') ? { url: 'https://x/#k' } : []; },
  };
  const mint = method('space-links', 'mint', {
    ClipboardItem: class {}, Blob: class {}, navigator: { clipboard: { write: async () => {} } },
  });
  await mint(mintSelf, 'bafy-a');
  await mint(mintSelf);
  const bodies = posted.filter(([path]) => path.endsWith('/invite')).map(([, body]) => body);
  assert.deepEqual(bodies, [{ replaces: 'bafy-a' }, {}]);
});

test('the copy note shows only while no listed link can be copied', () => {
  const note = (made) => {
    const self = linksSelf({ made: () => made, _rows: [{ kind: 'open', target: 'bafy-a', default: true, used: [] }, { kind: 'open', target: 'bafy-b', used: [] }] });
    self.esc = (text) => method('space-links', 'esc')(self, text);
    self.names = (list) => method('space-links', 'names')(self, list);
    self.shown = (url) => method('space-links', 'shown')(self, url);
    self.askHtml = () => '';
    self.querySelector = () => null;
    method('space-links', 'render', { Intl })(self);
    return /Only a link made in this tab can be copied/.test(self.innerHTML);
  };
  assert.equal(note(new Map()), true);
  assert.equal(note(new Map([['bafy-b', 'https://tonk.example/space/x#key']])), false);
});

test('a passlink row shows its host and the start of its key, never the key', () => {
  const shown = method('space-links', 'shown')({}, 'https://tonk.example/space/did:key:z6Mk#abcdefSECRET');
  assert.equal(shown, 'tonk.example/…#abc•••');
});

test('a waiting button takes no second press at once and says so only past 400ms', () => {
  const timers = [];
  const waiting = method('hub-discover', 'waiting', { setTimeout: (run, ms) => { timers.push({ run, ms }); return timers.length; }, clearTimeout: () => {} });
  const attrs = new Map();
  const button = {
    innerHTML: 'remove link', textContent: 'remove link', style: {},
    getBoundingClientRect: () => ({ width: 80 }),
    setAttribute: (name, value) => attrs.set(name, value),
    getAttribute: (name) => attrs.get(name),
    removeAttribute: (name) => attrs.delete(name),
  };
  const done = waiting({}, button, 'removing');
  assert.equal(attrs.get('aria-disabled'), 'true');
  assert.equal(button.textContent, 'remove link');
  assert.equal(timers[0].ms, 400);
  timers[0].run();
  assert.equal(button.textContent, 'removing');
  assert.equal(attrs.get('aria-busy'), 'true');
  done();
  assert.equal(attrs.has('aria-busy'), false);
  assert.equal(button.innerHTML, 'remove link');
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

test('the space menu steps out of sight under its delete dialog and closes with it', () => {
  const connected = methodSource('space-remove', 'connected');
  assert.match(connected, /setAttribute\('data-behind', ''\)/);
  assert.match(connected, /'fabb-close'/);
  assert.match(connected, /menu\.close\?\.\(\)/);
  assert.match(library, /\.space-menu\[data-behind\][^{]*\{[^}]*visibility:hidden/);
  assert.match(library, /\.space-menu\[data-behind\] tonk-dialog \{ visibility:visible; \}/);
});
