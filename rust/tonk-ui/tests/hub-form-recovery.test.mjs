import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
function method(element, name) {
  const section = library.split(`element!: &${element}\n`)[1];
  const source = section.split(`    ${name}: |\n`)[1].split(/\n    [^ ]|\n[^ ]/)[0];
  return Function(`return (${source.trim()})`)();
}
function attributes(object = {}) {
  const values = new Map();
  return Object.assign(object, {
    setAttribute: (key, value) => values.set(key, value),
    getAttribute: key => values.get(key),
    hasAttribute: key => values.has(key),
    removeAttribute: key => values.delete(key),
  });
}
function fixture(kind) {
  const name = attributes({ value: 'Ada', defaultValue: 'Ada', focus() {} });
  const error = { textContent: '', hidden: true };
  const submit = { disabled: false };
  const form = attributes({
    elements: { name, description: { value: 'Notes' }, open: { value: 'true' } },
    matches: () => true,
    closest: () => form,
    querySelector: selector => selector.includes('error') ? error : submit,
  });
  const self = attributes({
    isConnected: true,
    querySelector: selector => selector.includes('error') ? error : selector.includes('dialog') ? { close() {} } : form,
    querySelectorAll: () => [submit],
  });
  let pending;
  const element = kind === 'create' ? 'space-create' : 'account-settings';
  self.watch = query => method(element, 'watch')(self, query);
  self.claim = request => method(element, 'claim')(self, request);
  self.create = form => { pending = method('space-create', 'create')(self, form); };
  self.profileNameSave = (form, name) => { pending = method('account-settings', 'profile-name-save')(self, form, name); };
  const event = { target: form, prevented: false, stopped: false,
    preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } };
  return { self, form, name, error, submit, event, pending: () => pending,
    run: () => method(kind === 'create' ? 'space-create' : 'account-settings', kind === 'create' ? 'validate' : 'profile-name-submit')(self, event) };
}
function bridge(outcome, { reject = false, ended = false } = {}) {
  let id;
  let calls = 0;
  let cancelled = 0;
  let sent;
  globalThis.window = { tonk: { ready: Promise.resolve(), context: {}, navigate() {} } };
  const frame = rows => new TextEncoder().encode(`data: ${JSON.stringify(rows)}\n\n`);
  globalThis.fetch = async (url, init) => {
    const body = JSON.parse(init.body);
    if (url.endsWith('/query')) {
      id = body.terms.this;
      init.signal.addEventListener('abort', () => { cancelled++; });
      return new Response(new ReadableStream({
        start(controller) {
          if (!ended) {
            controller.enqueue(frame([{ this: 'urn:uuid:another-request', fields: { status: 'failed', detail: 'Unrelated failure' } }]));
            controller.enqueue(frame([{ this: id, fields: outcome }]));
          }
          controller.close();
        },
      }));
    }
    assert.ok(url.endsWith('/transact'), `unexpected request to ${url}`);
    calls++;
    sent = body.claims[0].application.parameters;
    assert.equal(sent.this, id, 'subscribe before dispatch with the same request ID');
    if (reject) throw new Error('Transport failed');
    return new Response('{}');
  };
  return { calls: () => calls, cancelled: () => cancelled, sent: () => sent };
}

test('Discover submits a remote catalog reference and preserves retry after refusal', async () => {
  const { self, form, error, submit, run, pending } = fixture('create');
  form.elements.template = { value: 'https://example.com/catalog.json#demo' };
  const failed = bridge({ status: 'failed', detail: 'Template could not be read' });
  window.tonk.context = { origin: 'https://tonk.example' };
  run();
  await pending();
  assert.equal(failed.sent().template, 'https://example.com/catalog.json#demo');
  assert.equal(error.textContent, 'Template could not be read');
  assert.equal(form.elements.name.value, 'Ada');
  assert.equal(submit.disabled, false);
  assert.equal(self.hasAttribute('busy'), false);
  const success = bridge({ status: 'created', detail: '/space/new-copy' });
  window.tonk.context = { origin: 'https://tonk.example' };
  let destination;
  window.tonk.navigate = path => { destination = path; };
  run();
  run();
  await pending();
  assert.equal(success.calls(), 1, 'a pending copy ignores a second submit');
  assert.equal(destination, '/space/new-copy');
});

test('unchanged display name is a no-op and a subsequent edit can submit', async () => {
  const f = fixture('rename');
  const b = bridge({ status: 'renamed' });
  f.run();
  assert.equal(f.submit.disabled, false);
  assert.equal(f.pending(), undefined);
  assert.ok(f.event.prevented && f.event.stopped);
  f.name.value = 'Grace';
  f.run();
  assert.equal(f.submit.disabled, true);
  await f.pending();
  assert.equal(b.calls(), 1);
  assert.equal(f.name.defaultValue, 'Grace');
  assert.equal(f.submit.disabled, false);
  assert.notEqual(f.form.getAttribute('aria-busy'), 'true');
});

for (const kind of ['create', 'rename']) {
  for (const failure of ['receipt', 'transport', 'stream']) {
    test(`${kind} recovers from ${failure} failure, preserves input and allows retry`, async () => {
      const f = fixture(kind);
      f.name.value = 'Grace';
      const b = bridge({ status: 'failed', detail: 'Please try again.' }, {
        reject: failure === 'transport', ended: failure === 'stream',
      });
      f.run();
      f.run();
      assert.equal(f.submit.disabled, true);
      await f.pending();
      assert.equal(b.calls(), 1, 'repeat submits are blocked');
      assert.equal(b.cancelled(), 1);
      assert.equal(f.name.value, 'Grace');
      assert.equal(f.submit.disabled, false);
      assert.notEqual(f.form.getAttribute('aria-busy'), 'true');
      assert.ok(f.error.textContent);
      if (failure === 'receipt') assert.equal(f.error.textContent, 'Please try again.');
      const retry = bridge({ status: kind === 'create' ? 'created' : 'renamed', detail: '/space/example' });
      f.run();
      await f.pending();
      assert.equal(retry.calls(), 1);
      assert.equal(f.error.textContent, '');
      if (kind === 'create') assert.equal(retry.sent().description, 'Notes');
    });
  }
}

test('settings asks the worker on its own origin, not the app page\'s', async () => {
  const asked = [];
  const before = { window: globalThis.window };
  globalThis.window = {
    fetch: async (url, init) => {
      asked.push([url, init?.method ?? 'GET']);
      return { ok: true, text: async () => '{"subject":"did:key:zSpace"}' };
    },
  };
  // What the page around says of itself: the app's origin, which holds no worker API.
  const self = { context: () => ({ origin: 'https://tonk.test', path: '/settings/link' }) };
  try {
    const described = await method('account-settings', 'api')(self, '/api/local-space-link/describe', { request: 'r' });
    assert.deepEqual(described, { subject: 'did:key:zSpace' });
    assert.deepEqual(asked, [['/api/local-space-link/describe', 'POST']]);
  } finally {
    globalThis.window = before.window;
  }
});

// The hub's door, with the page stood in for: where it is, whether an
// account is linked, and whether the worker holds a stage of the panel.
function door({ path, linked = false, staged = false, linking = false }) {
  const calls = [];
  const self = attributes({
    linked: () => linked,
    unlinked: () => !linked,
    staged: () => staged,
    viaRequest: () => null,
    viaAsk: () => false,
    context: () => ({ path }),
    connectionsRefresh: () => calls.push('connections'),
    openRegistration: () => calls.push('open'),
    openSignInVia: () => calls.push('open-via'),
    leaveRegistration: () => calls.push('leave'),
    unlink: () => calls.push('unlink'),
  });
  if (linking) self.setAttribute('data-linking', 'true');
  const before = { document: globalThis.document, window: globalThis.window };
  globalThis.document = { querySelector: () => null };
  globalThis.window = { tonk: { navigate: (to) => calls.push(`navigate ${to}`) } };
  try {
    method('account-settings', 'door')(self);
  } finally {
    globalThis.document = before.document;
    globalThis.window = before.window;
  }
  return calls;
}

test('a stage left from another visit is put away off the account page', () => {
  assert.deepEqual(door({ path: '/', staged: true }), ['leave']);
  assert.deepEqual(door({ path: '/', linked: true, staged: true }), ['leave']);
});

test('the account page keeps the panel, and opens it when none is up', () => {
  assert.deepEqual(door({ path: '/account', staged: true, linking: true }), []);
  assert.deepEqual(door({ path: '/account' }), ['open']);
});

test('a page with no stage showing leaves the panel alone', () => {
  assert.deepEqual(door({ path: '/' }), []);
});
