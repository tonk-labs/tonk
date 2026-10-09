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
  const self = attributes({
    request: { ...request },
    linkRequest() { return this.request; },
    querySelector: selector => selector === '[data-ceremony-row]' ? row
      : selector === '[data-ceremony-status]' ? line
      : selector.startsWith('tonk-display') ? progress : null,
    localLinkShow: () => false, viaShow: () => false, text() {}, pane() {}, seat() {},
    context: () => ({ hash: '' }), connectionsRefresh() {},
  });
  for (const name of ['base58', 'approval-entity', 'ceremony-target', 'refresh', 'describe', 'status']) {
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

test('current request reports progress and a fast terminal result without observing pending', () => {
  for (const [state, text] of [
    ['pending-ceremony', 'Approving the connection: waiting for passkey…'],
    ['working', 'Approving the connection…'],
    ['done', 'Approved. Completing the connection…'],
    ['refused', 'Approving the connection did not finish: refused callback'],
    ['failed', 'Approving the connection did not finish: refused callback'],
  ]) {
    const { self, row, line } = fixture();
    self.refresh();
    row.dataset = { of: self.ceremonyTarget(), ceremony: 'authorize-device', ceremonyState: state, ceremonyDetail: 'refused callback' };
    self.describe();
    assert.equal(line.textContent, text);
    assert.equal(line.hidden, false);
  }
});

test('switching requests clears old status and ignores delayed results from another tab', () => {
  const { self, row, line } = fixture();
  self.refresh();
  row.dataset.of = self.ceremonyTarget();
  self.describe();
  assert.equal(line.hidden, false);
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
  assert.equal(second.line.textContent, 'Approving the connection…');
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
