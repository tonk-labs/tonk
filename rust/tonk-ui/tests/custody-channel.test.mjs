import assert from 'node:assert/strict';
import {test} from 'node:test';
import {waitForCustodyReply, startCustodyProgress} from '../../tonk-identity/src/custody-channel.mjs';

const deliver = () => new Promise(setImmediate);
const timeoutMessage = /did not answer the custody handoff in time/;

test('silent worker times out and releases the reply listener', async t => {
  t.mock.timers.enable({apis: ['setTimeout']});
  const {port1, port2} = new MessageChannel();
  t.after(() => port2.close());
  const result = assert.rejects(waitForCustodyReply(port1, 30), timeoutMessage);
  t.mock.timers.tick(30);
  await result;
  assert.equal(port1.onmessage, null);
  assert.equal(port1.onmessageerror, null);
});

test('without progress, a late successful operation loses its callback', async t => {
  t.mock.timers.enable({apis: ['setTimeout']});
  const {port1, port2} = new MessageChannel();
  t.after(() => port2.close());
  const result = assert.rejects(waitForCustodyReply(port1, 30), timeoutMessage);
  t.mock.timers.tick(30);
  await result;
  port2.postMessage({ok: {navigate: 'https://example.test/callback#test'}});
  await deliver();
  assert.equal(port1.onmessage, null);
});

test('worker progress retains a slow operation until its terminal callback', async t => {
  t.mock.timers.enable({apis: ['setTimeout', 'setInterval']});
  const {port1, port2} = new MessageChannel();
  t.after(() => port2.close());
  let settled = false;
  const reply = waitForCustodyReply(port1, 30).then(value => { settled = true; return value; });
  const stop = startCustodyProgress(port2, 10);
  t.after(stop);
  await deliver();
  for (let i = 0; i < 12; i++) {
    t.mock.timers.tick(10);
    await deliver();
    assert.equal(settled, false, 'progress must not be interpreted as completion');
  }
  const terminal = {ok: {navigate: 'https://example.test/callback#test'}};
  port2.postMessage(terminal);
  stop();
  assert.deepEqual(await reply, terminal);
  assert.equal(port1.onmessage, null);
});

test('worker disappearing after progress still times out', async t => {
  t.mock.timers.enable({apis: ['setTimeout', 'setInterval']});
  const {port1, port2} = new MessageChannel();
  t.after(() => port2.close());
  const result = assert.rejects(waitForCustodyReply(port1, 30), timeoutMessage);
  const stop = startCustodyProgress(port2, 10);
  await deliver();
  stop();
  t.mock.timers.tick(30);
  await result;
  assert.equal(port1.onmessage, null);
});

test('a terminal refusal preserves its structured code after progress', async t => {
  const {port1, port2} = new MessageChannel();
  t.after(() => port2.close());
  const result = assert.rejects(waitForCustodyReply(port1, 1000), {
    message: 'Confirmation required', code: 'AWAITING_ACTIVATION',
  });
  const stop = startCustodyProgress(port2);
  port2.postMessage({error: 'Confirmation required', code: 'AWAITING_ACTIVATION'});
  stop();
  await result;
  assert.equal(port1.onmessage, null);
});

test('the worker guard stops all progress when its operation finishes', t => {
  t.mock.timers.enable({apis: ['setInterval']});
  const messages = [];
  const stop = startCustodyProgress({postMessage: message => messages.push(message)}, 10);
  t.mock.timers.tick(20);
  assert.equal(messages.length, 3);
  assert.deepEqual(messages[0], {pending: true});
  stop();
  t.mock.timers.tick(1000);
  assert.equal(messages.length, 3);
});
