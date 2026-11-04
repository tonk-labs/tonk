// "Your agents": revoking a passlink grant asks for the passkey, and the
// space stays listed until the worker reports the revocation acknowledged.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
function method(name) {
  const section = library.split('element!: &hub-discover\n')[1];
  const source = section.split(`    ${name}: |\n`)[1].split(/\n    [^ ]|\n[^ ]/)[0];
  return Function(`return (${source.trim()})`)();
}

const AGENT = 'did:key:agent';
const SPACE = 'did:key:space';

// A node standing in for the hub: the status line, the list, and the
// methods under test bound to it.
function fixture(answers) {
  const status = { textContent: '' };
  const rows = [];
  const list = {
    set textContent(_) { rows.length = 0; },
    append: node => rows.push(node),
  };
  const self = {
    isConnected: true,
    getAttribute: () => 'contacts',
    querySelector: selector => selector === '[data-contacts-status]' ? status
      : selector === '[data-contacts-list]' ? list : null,
    agents: async () => {},
  };
  for (const name of ['agents-revoke', 'agents-watch', 'agents-render']) {
    self[name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = (...args) => method(name)(self, ...args);
  }
  const calls = [];
  globalThis.window = { location: { origin: 'https://tonk.test' } };
  globalThis.fetch = async (url, init) => {
    calls.push([url, init?.body ?? null]);
    const answer = answers.shift();
    return { ok: true, status: 200, json: async () => answer, text: async () => JSON.stringify(answer) };
  };
  globalThis.document = {
    createElement: () => {
      const node = { dataset: {}, children: [], className: '', textContent: '', append(child) { this.children.push(child); }, setAttribute() {} };
      return node;
    },
  };
  return { self, status, calls, rows };
}

function button() {
  return { dataset: { agentRevoke: 'agent-key', agentSpace: SPACE, agentLink: 'true' }, textContent: 'revoke', disabled: false };
}

const listed = removal => [{ did: AGENT, key: 'agent-key', name: 'Claude', spaces: [{ subject: SPACE, name: 'Trip', access: 'read', viaLink: true, ...(removal ? { removal } : {}) }] }];

test('the first press says the passkey is asked for and the link stays usable', async () => {
  const { self, status, calls } = fixture([]);
  const revoke = button();
  await self.agentsRevoke(revoke);
  assert.equal(calls.length, 0, 'nothing is sent before confirming');
  assert.equal(revoke.textContent, 'confirm revoke');
  assert.match(status.textContent, /passkey/);
  assert.match(status.textContent, /does not end the passlink/);
});

test('a pending removal is watched until the space leaves the list', async () => {
  const { self, status, calls } = fixture([{ pending: true }, listed({ state: 'working' }), []]);
  const timers = [];
  globalThis.setTimeout = fn => timers.push(fn);
  const revoke = button();
  revoke.dataset.confirming = 'true';
  await self.agentsRevoke(revoke);
  assert.deepEqual(JSON.parse(calls[0][1]), { space: SPACE });
  assert.equal(revoke.textContent, 'waiting for passkey…');
  assert.match(status.textContent, /Use your passkey/);
  await new Promise(resolve => setImmediate(resolve));
  assert.match(status.textContent, /Revoking/, 'still listed while working');
  assert.equal(timers.length, 1);
  await timers.shift()();
  assert.match(status.textContent, /Access revoked/);
  assert.match(status.textContent, /passlink still works/);
});

test('a failed removal keeps the space listed with its reason and a retry', async () => {
  const { self, status, rows } = fixture([listed({ state: 'failed', reason: 'the access service did not answer' })]);
  globalThis.setTimeout = () => assert.fail('a failure is not watched further');
  await self.agentsWatch('agent-key', SPACE, Date.now());
  assert.match(status.textContent, /did not finish, so it is still listed: the access service did not answer/);
  const space = rows[0].children.find(child => child.className === 'contact-spaces').children[0];
  const control = space.children.find(child => child.dataset.agentRevoke);
  assert.equal(control.textContent, 'retry revoke');
  assert.equal(control.disabled, false);
});

test('a dismissed passkey leaves the access listed to try again', async () => {
  const { self, status } = fixture([listed(null)]);
  globalThis.setTimeout = () => assert.fail('nothing is left to watch');
  await self.agentsWatch('agent-key', SPACE, Date.now());
  assert.match(status.textContent, /did not finish, so the access is still listed/);
});
