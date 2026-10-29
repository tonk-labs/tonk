import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createAgentViews } from './agent-views.mjs';

const agent = 'a'.repeat(32);

// A served agent without a process: it records calls and requests, and
// answers like the worker.
function fakeServe(log) {
  return async ({ directory }) => {
    log.started = directory;
    log.files = await readdir(directory);
    return {
      async call(call, args) {
        log.calls.push([call, args]);
        return { call };
      },
      async request(path, options = {}) {
        log.requests.push([options.method ?? 'GET', path]);
        if (path.endsWith('/sync/push')) return new Response(null, { status: log.push ?? 200 });
        if (path.endsWith('/query'))
          return new Response(
            new ReadableStream({
              start(controller) {
                log.stream = controller;
              },
            }),
            { headers: { 'content-type': 'text/event-stream' } },
          );
        return Response.json({ ok: true });
      },
      async close() {
        log.closed = (log.closed ?? 0) + 1;
      },
    };
  };
}

async function setup(t, options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-agent-views-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(join(directory, 'agents', agent, 'peer'), { recursive: true });
  await writeFile(join(directory, 'agents', agent, 'sponsor.json'), '{}');
  await writeFile(join(directory, 'agents', agent, 'peer.lock'), '');
  const log = { calls: [], requests: [], exclusive: 0 };
  let clock = 0;
  const views = createAgentViews({
    binary: 'unused',
    directory,
    remote: 'https://tonk.example/ucan/',
    exclusive: async (task) => {
      log.exclusive++;
      return task();
    },
    start: fakeServe(log),
    now: () => clock,
    ...options,
  });
  t.after(() => views.close());
  return { views, log, tick: (ms) => (clock += ms) };
}

test('an open agent runs on a copy of its directory, without the lock', async (t) => {
  const { views, log } = await setup(t);
  assert.equal(views.running(agent), undefined);
  const served = await views.open(agent);
  assert.equal(log.exclusive, 1, 'the copy is taken in the agent store queue');
  assert.deepEqual(log.files.sort(), ['peer', 'sponsor.json']);
  assert.equal(await views.running(agent), served);
  assert.equal(await views.open(agent), served, 'a second view reuses the process');
  assert.equal(log.exclusive, 1);
  await served.call('tonk_query', { space: 'did:key:z', document: 'x:' });
  assert.deepEqual(log.calls, [['tonk_query', { space: 'did:key:z', document: 'x:' }]]);
  await views.stop(agent);
  assert.equal(log.closed, 1);
  assert.equal(views.running(agent), undefined);
  await assert.rejects(stat(log.started), 'the copy is removed');
});

test('a write from a view is pushed at once, and an unconfirmed push is reported', async (t) => {
  const { views, log } = await setup(t);
  const served = await views.open(agent);
  const base = '/api/repository/did%3Akey%3Az/branch/main';
  const written = await served.request(base + '/transact', { method: 'POST', body: '{}' });
  assert.equal(written.status, 200);
  assert.deepEqual(log.requests.at(-1), ['POST', base + '/sync/push']);

  const read = await served.request(base + '/evaluate?transact=false', { method: 'POST' });
  await read.text();
  assert.deepEqual(log.requests.at(-1), ['POST', base + '/evaluate?transact=false']);

  log.push = 500;
  const uncertain = await served.request(base + '/evaluate?transact=true', { method: 'POST' });
  assert.equal(uncertain.status, 503);
  assert.match((await uncertain.json()).error, /push could not be confirmed/);
});

test('an agent closes when idle, but not while a view still streams', async (t) => {
  const { views, log, tick } = await setup(t, { idle: 1000 });
  const served = await views.open(agent);
  const stream = await served.request('/api/repository/did%3Akey%3Az/branch/main/query', {
    method: 'POST',
  });
  const reader = stream.body.getReader();
  tick(5000);
  await views.sweepIdle();
  assert.ok(views.running(agent), 'a streaming view keeps the agent open');
  log.stream.close();
  assert.equal((await reader.read()).done, true);
  await views.sweepIdle();
  assert.ok(views.running(agent), 'the stream just ended');
  tick(5000);
  await views.sweepIdle();
  assert.equal(views.running(agent), undefined);
  assert.equal(log.closed, 1);
});

test('a failed start leaves nothing open', async (t) => {
  const { views } = await setup(t, {
    start: async () => {
      throw Error('no');
    },
  });
  await assert.rejects(views.open(agent));
  assert.equal(views.running(agent), undefined);
});

test('only agent ids open', async (t) => {
  const { views } = await setup(t);
  assert.throws(() => views.open('../x'));
});
