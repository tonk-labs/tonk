// WEB-10: approval progress belongs to one audience/callback, including remounts.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const library = readFileSync(new URL('../../tonk-core/assets/library/profile.yaml', import.meta.url), 'utf8');
function method(name) {
  const section = library.split('element!: &account-settings\n')[1];
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
const request = { audience: 'did:key:test', callback: 'https://example.test/callback?request=1' };
function fixture() {
  const progress = attributes();
  progress.setAttribute('entity', 'state:ceremony');
  const line = { textContent: '', hidden: true };
  const row = { dataset: { of: 'state:ceremony', ceremony: 'authorize-device', ceremonyState: 'done' } };
  // The device task page's own parts, recorded rather than drawn.
  const task = { waiting: false, step: 'ask', error: '' };
  const self = attributes({
    task,
    wait: () => { task.waiting = true; },
    settle: () => { task.waiting = false; },
    linkStep: step => { task.step = step; },
    linkError: text => { task.error = text; },
    request: { ...request },
    linkRequest() { return this.request; },
    querySelector: selector => selector === '[data-ceremony-row]' ? row
      : selector === '[data-ceremony-status]' ? line
      : selector.startsWith('tonk-display') ? progress : null,
    localLinkShow: () => false, viaShow: () => false, agentShow: () => false, text() {}, pane() {}, seat() {},
    context: () => ({ hash: '', path: '/settings/link', search: '' }), connectionsRefresh() {},
  });
  for (const name of ['base58', 'approval-entity', 'ceremony-target', 'link-outcome', 'refresh', 'describe', 'status']) {
    self[name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = (...args) => method(name)(self, ...args);
  }
  return { self, row, progress, line };
}

test('fresh approval hides an earlier completed or failed ceremony', () => {
  for (const state of ['done', 'failed', 'working', 'pending-ceremony']) {
    const { self, row, line, progress } = fixture();
    row.dataset.ceremonyState = state;
    self.refresh(); self.describe();
    assert.equal(line.hidden, true);
    assert.equal(self.hasAttribute('data-passkey-requested'), false);
    assert.equal(progress.getAttribute('entity'), self.approvalEntity(request));
  }
});

// The device task page answers for its own request; the status line under
// the hub stays quiet.
test('current request reports progress and a fast terminal result without observing pending', () => {
  for (const [state, detail, waiting, step, error] of [
    ['pending-ceremony', '', true, 'ask', ''],
    ['working', '', true, 'ask', ''],
    ['done', '', false, 'ask', ''],
    ['refused', 'refused callback', false, 'ask', /passkey prompt was cancelled/],
    // The mock's #device-failed words; the worker's detail goes to the console.
    ['failed', 'refused callback', false, 'ask', /^Tonk couldn’t reach the device, so nothing was connected\. Check that it’s still waiting, then try again\.$/],
    ['failed', 'request expired', false, 'expired', ''],
    ['failed', 'this handoff requires account did:key:a; the unlocked passkey belongs to did:key:b', false, 'ask', /requires account did:key:a/],
  ]) {
    const { self, row, line } = fixture();
    self.refresh();
    row.dataset = { of: self.ceremonyTarget(), ceremony: 'authorize-device', ceremonyState: state, ceremonyDetail: detail };
    self.describe();
    assert.equal(self.task.waiting, waiting, state);
    assert.equal(self.task.step, step, state);
    if (error) assert.match(self.task.error, error); else assert.equal(self.task.error, '');
    assert.equal(self.getAttribute('data-ceremony-state'), state);
    assert.equal(line.hidden, true);
  }
});

test('switching requests clears old status and ignores delayed results from another tab', () => {
  const { self, row, line } = fixture();
  self.refresh();
  row.dataset.of = self.ceremonyTarget();
  self.describe();
  assert.equal(self.getAttribute('data-ceremony-state'), 'done');
  self.request.callback += '2';
  self.refresh(); self.describe();
  assert.equal(line.hidden, true);
  assert.equal(self.hasAttribute('data-ceremony-state'), false);
});

test('a remounted panel resumes its own approval without another click', () => {
  const first = fixture();
  first.self.refresh();
  const second = fixture();
  second.self.refresh();
  second.row.dataset = { of: first.self.ceremonyTarget(), ceremony: 'authorize-device', ceremonyState: 'working' };
  second.self.describe();
  assert.equal(second.self.task.waiting, true);
  assert.equal(second.self.hasAttribute('data-passkey-requested'), true);
});

test('non-approval ceremonies retain their normal progress row', () => {
  const { self, row, line } = fixture();
  self.request = null;
  self.refresh();
  row.dataset = { of: 'state:ceremony', ceremony: 'delete-account', ceremonyState: 'done' };
  self.describe();
  assert.equal(line.textContent, 'Account deleted.');
});

test('approval entity uses the worker encoding and changes with either request field', () => {
  const { self } = fixture();
  // Shared vector in router/ceremony.rs; UTF-8 JSON array encoded as base58.
  assert.equal(self.approvalEntity(request), 'urn:tonk:approval:q5ieDH7i7UxzSkRNRGzuyDmYp5EvkdFMK3Xbzs7kzj2N354oyxRSb5xCqx2pBRNmmPRTa3UCjAE5Byv');
  assert.notEqual(self.approvalEntity(request), self.approvalEntity({ ...request, audience: 'did:key:other' }));
  assert.notEqual(self.approvalEntity(request), self.approvalEntity({ ...request, callback: request.callback + '2' }));
});

// Before approval there is no request-specific row. That is an expected
// absence, not a malformed ceremony that should expose schema diagnostics.
test('an approval without a ceremony row has a quiet absence fallback', () => {
  const display = library.match(/<tonk-display model="state:ceremony"[^>]*>([\s\S]*?)<\/tonk-display>/);
  assert.ok(display, 'settings contains its ceremony display');
  assert.match(display[1], /<span slot="no-entity" hidden><\/span>/);
});

// The rows observer re-runs describe on every write under the element, so a
// repeated answer must not write the error line again.
test('the device error line is written once for a repeated answer', () => {
  let writes = 0;
  let text = '';
  const error = { hidden: true, focus() {} };
  Object.defineProperty(error, 'textContent', { get: () => text, set: value => { writes += 1; text = value; } });
  const self = { querySelector: selector => selector === '[data-link-error]' ? error : null };
  const linkError = method('link-error');
  linkError(self, 'Tonk couldn’t finish.');
  linkError(self, 'Tonk couldn’t finish.');
  assert.equal(writes, 1);
  assert.equal(error.hidden, false);
  linkError(self, '');
  linkError(self, '');
  assert.equal(writes, 2);
  assert.equal(error.hidden, true);
});
