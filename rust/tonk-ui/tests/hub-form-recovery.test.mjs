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
  globalThis.window = { tonk: {
    ready: Promise.resolve(),
    subscribe(query) {
      id = query.terms.this;
      let reads = 0;
      return { getReader: () => ({
        read: async () => ended ? { done: true } : { value: reads++ === 0
          ? [{ this: 'urn:uuid:another-request', fields: { status: 'failed', detail: 'Unrelated failure' } }]
          : [{ this: id, fields: outcome }] },
        cancel: async () => { cancelled++; },
      }) };
    },
    transact: async command => {
      calls++;
      sent = command.claims[0].application.parameters;
      assert.equal(sent.this, id, 'subscribe before dispatch with the same request ID');
      if (reject) throw new Error('Transport failed');
    },
    navigate() {},
  } };
  return { calls: () => calls, cancelled: () => cancelled, sent: () => sent };
}

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
