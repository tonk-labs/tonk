import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(
  new URL('../../rust/tonk-portal/src/bootstrap.js', import.meta.url),
  'utf8',
);

async function selectText(selectionReporting) {
  const listeners = new Map();
  const sent = [];
  let timer;
  const port = { postMessage: (message) => sent.push(message) };
  const window = { fetch() {} };
  runInNewContext(source, {
    window,
    document: {
      addEventListener: (name, callback) => listeners.set(name, callback),
      getSelection: () => 'Selected heading',
    },
    parent: { postMessage() {} },
    MessageChannel: class {
      port1 = port;
      port2 = {};
    },
    setTimeout: (callback) => {
      timer = callback;
      return 1;
    },
    clearTimeout() {},
  });
  port.onmessage({ data: { type: 'ready', context: { site: 'id:tab', selectionReporting } } });
  listeners.get('selectionchange')();
  timer();
  await Promise.resolve();
  return sent;
}

test('embedded selection opt-out prevents transactions while ordinary guests still report', async () => {
  assert.equal((await selectText(false)).length, 0);
  for (const setting of [undefined, true]) {
    const messages = await selectText(setting);
    assert.equal(messages.length, 1);
    assert.equal(messages[0].type, 'transact');
    assert.equal(messages[0].request.claims[0].application.parameters.text, 'Selected heading');
  }
});
