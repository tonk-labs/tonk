// The agent join page follows the `grant-agent` ceremony row to the end,
// even when the passkey finishes before the join request answers.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
function method(name) {
  const section = library.split('element!: &account-settings\n')[1];
  const source = section.split(`    ${name}: |\n`)[1].split(/\n    [^ ]|\n[^ ]/)[0];
  return Function(`return (${source.trim()})`)();
}

const SPACE = 'did:key:z6MkSpace';

function fixture(answer) {
  const attributes = new Map();
  const panels = ['ask', 'joining', 'passkey', 'working', 'done', 'error'].map((step) => ({
    dataset: { joinPanel: step },
    hidden: step !== 'ask',
  }));
  const pane = {
    dataset: { joinStep: 'ask' },
    querySelectorAll: (selector) => (selector === '[data-join-panel]' ? panels : []),
  };
  const row = { dataset: { of: 'state:ceremony', ceremony: 'grant-agent', ceremonyState: 'pending-ceremony', ceremonyDetail: '' } };
  const texts = {};
  let release;
  const answered = new Promise((resolve) => { release = resolve; });
  let listed = [];
  const self = {
    setAttribute: (key, value) => attributes.set(key, value),
    getAttribute: (key) => attributes.get(key),
    hasAttribute: (key) => attributes.has(key),
    removeAttribute: (key) => attributes.delete(key),
    context: () => ({ path: '/settings/agent-join', hash: `#agent=did%3Akey%3Az6MkAgent&link=${encodeURIComponent('https://tonk.example/space/x#seed')}` }),
    querySelector: (selector) => selector === '[data-agent-join]' ? pane
      : selector === '[data-ceremony-row]' ? row
      : selector.includes('aria-checked') ? { dataset: { access: 'read' } }
      : null,
    ceremonyTarget: () => 'state:ceremony',
    text: (selector, value) => { texts[selector] = value; },
    status() {},
    api: (path) => path === '/api/agents' ? Promise.resolve(listed) : answered.then(() => answer),
    list(agents) { listed = agents; },
  };
  for (const name of ['agent-join-request', 'agent-join-step', 'agent-join-error', 'agent-join-go', 'agent-join-ceremony', 'agent-join-watch', 'agent-join-done', 'describe']) {
    self[name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = (...args) => method(name)(self, ...args);
  }
  self._joinAgent = 'Claude';
  return { self, pane, row, texts, release };
}

test('a ceremony that ends before the join answers still shows done', async () => {
  const { self, pane, row, texts, release } = fixture({ pending: true, space: SPACE, subject: SPACE });
  self.agentJoinGo();
  assert.equal(pane.dataset.joinStep, 'joining');
  // The passkey and the grant finish while the request is still out.
  Object.assign(row.dataset, { ceremonyState: 'done', ceremonyDetail: SPACE });
  self.describe();
  assert.equal(pane.dataset.joinStep, 'joining', 'the space is not known yet');
  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(pane.dataset.joinStep, 'done');
  assert.match(texts['[data-join-done]'], /Claude can now read the space/);
});

test('an earlier join of another space does not count as done', async () => {
  const { self, pane, row, release } = fixture({ pending: true, space: SPACE, subject: SPACE });
  Object.assign(row.dataset, { ceremonyState: 'done', ceremonyDetail: 'did:key:z6MkOther' });
  self.agentJoinGo();
  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(pane.dataset.joinStep, 'passkey');
  // Leave the page: the watch for the grant stops.
  self._joinAsked = false;
});

// The ceremony row may never show done; the page still finishes once the
// agent holds the space.
test('the page finishes when the agent holds the space, without a done row', async () => {
  const { self, pane, row, texts, release } = fixture({ pending: true, space: SPACE, subject: SPACE });
  self.list([{ key: 'did:key:z6MkAgent', spaces: [{ subject: SPACE, access: 'read' }] }]);
  Object.assign(row.dataset, { ceremonyState: 'working', ceremonyDetail: SPACE });
  self.agentJoinGo();
  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(pane.dataset.joinStep, 'done');
  assert.match(texts['[data-join-done]'], /Claude can now read the space/);
});
