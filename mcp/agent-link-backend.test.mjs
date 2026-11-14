import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { agentLinkBackend, invoke } from './agent-link-backend.mjs';
import { createTonkHTTP } from './http.mjs';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('subprocess failures emit bounded secret-safe diagnostics after close', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-agent-diagnostics-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const secret = 'private-diagnostic-fixture';
  const cases = [
    { category: 'exit', source: 'process.exitCode = 7;', status: 7, signal: null },
    {
      category: 'exit',
      source: "process.kill(process.pid, 'SIGTERM');",
      status: null,
      signal: 'SIGTERM',
    },
    { category: 'missing_result', source: 'console.log(text);', status: 0, signal: null },
    {
      category: 'invalid_result',
      source: "console.log('TONK_AGENT_LINK_RESULT ' + text + 'broken');",
      status: 0,
      signal: null,
    },
    {
      category: 'output_limit',
      source: "process.stdout.write('x'.repeat(2100001)); setInterval(() => {}, 1000);",
    },
    { category: 'timeout', source: 'setInterval(() => {}, 1000);', timeoutMs: 150 },
    { category: 'cancelled', source: 'setInterval(() => {}, 1000);', cancel: true },
    { category: 'spawn_error', missing: true },
  ];
  for (const fixture of cases) {
    await t.test(fixture.category + (fixture.signal ?? ''), async (subtest) => {
      const logs = [];
      subtest.mock.method(console, 'error', (...args) => logs.push(args));
      const binary = join(directory, fixture.missing ? secret : 'worker.mjs');
      if (!fixture.missing) {
        await writeFile(
          binary,
          `#!${process.execPath}
let text = '';
for await (const chunk of process.stdin) text += chunk;
console.error(text);
${fixture.source}
`,
          { mode: 0o700 },
        );
      }
      const controller = new AbortController();
      const timer = fixture.cancel ? setTimeout(() => controller.abort(secret), 150) : undefined;
      try {
        await assert.rejects(
          invoke({
            binary,
            dataDirectory: directory,
            remote: 'https://example.invalid/' + secret,
            signal: controller.signal,
            timeoutMs: fixture.timeoutMs ?? 5000,
            input: { tool: 'tonk_evaluate', agent_link: secret, document: secret },
          }),
          (error) => {
            assert.match(error.message, /Do not repeat it; query the space first/);
            assert.ok(!error.message.includes(secret));
            return true;
          },
        );
      } finally {
        clearTimeout(timer);
      }
      assert.equal(logs.length, 1);
      assert.ok(!JSON.stringify(logs).includes(secret));
      const diagnostic = JSON.parse(logs[0][1]);
      assert.equal(diagnostic.category, fixture.category);
      if ('status' in fixture) assert.equal(diagnostic.status, fixture.status);
      if ('signal' in fixture) assert.equal(diagnostic.signal, fixture.signal);
    });
  }
});

test('subprocess sends credentials on stdin and consumes only its final result', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-agent-process-'));
  const binary = join(directory, 'worker.mjs');
  await writeFile(
    binary,
    `#!${process.execPath}
let text = '';
for await (const chunk of process.stdin) text += chunk;
const input = JSON.parse(text);
if (process.argv.some(arg => arg.includes('private-fixture'))) process.exit(2);
console.log('ordinary worker diagnostic');
console.log('TONK_AGENT_LINK_RESULT ' + JSON.stringify({ result: { title: 'café', tool: input.tool } }));
`,
    { mode: 0o700 },
  );
  try {
    const backend = agentLinkBackend({
      binary,
      dataDirectory: directory,
      remote: 'https://access.example/ucan/',
    });
    assert.deepEqual(
      await backend.call('tonk_query', {
        agent_link: 'https://tonk.example/agent#private-fixture',
        document: 'concept:\n',
      }),
      { title: 'café', tool: 'tonk_query' },
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('a dispatched write survives client disconnect and checkpoints before the next call', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-agent-disconnect-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const binary = join(directory, 'worker.mjs');
  await writeFile(
    binary,
    `#!${process.execPath}
for await (const chunk of process.stdin) {}
setTimeout(() => console.log('TONK_AGENT_LINK_RESULT {"result":{"published":true}}'), 100);
`,
    { mode: 0o700 },
  );
  let dispatched;
  const ready = new Promise((resolve) => {
    dispatched = resolve;
  });
  const events = [];
  const backend = agentLinkBackend({
    binary,
    dataDirectory: directory,
    remote: 'https://access.example/ucan/',
    run: (options) => {
      events.push(options.input.tool);
      const result = invoke(options);
      dispatched();
      return result;
    },
    checkpoint: {
      save: async () => {
        events.push('checkpoint');
      },
    },
  });
  const args = { agent_link: 'https://access.example/agent#fixture', document: 'concept:\n' };
  const controller = new AbortController();
  const write = backend.call('tonk_evaluate', args, controller.signal);
  await ready;
  controller.abort();
  const query = backend.call('tonk_query', args);
  assert.deepEqual(await write, { published: true });
  await query;
  assert.deepEqual(events, ['tonk_evaluate', 'checkpoint', 'tonk_query', 'checkpoint']);
  await backend.close();
});

test('disconnected queries and queued writes still cancel without dispatching another write', async () => {
  let dispatched;
  const ready = new Promise((resolve) => {
    dispatched = resolve;
  });
  const calls = [];
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async ({ input, signal }) => {
      calls.push(input.tool);
      dispatched();
      await new Promise((resolve) => signal.addEventListener('abort', resolve, { once: true }));
      signal.throwIfAborted();
    },
  });
  const args = { agent_link: 'https://access.example/agent#fixture', document: 'concept:\n' };
  const queryController = new AbortController();
  const query = backend.call('tonk_query', args, queryController.signal);
  await ready;
  const writeController = new AbortController();
  const write = backend.call('tonk_evaluate', args, writeController.signal);
  writeController.abort();
  queryController.abort();
  await assert.rejects(query, { name: 'AbortError' });
  await assert.rejects(write, { name: 'AbortError' });
  assert.deepEqual(calls, ['tonk_query']);
  await backend.close();
});

test('graceful shutdown finishes the active write and checkpoint but rejects queued work', async () => {
  let dispatched, finish;
  const ready = new Promise((resolve) => {
    dispatched = resolve;
  });
  const hold = new Promise((resolve) => {
    finish = resolve;
  });
  let calls = 0,
    saved = 0;
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async ({ signal }) => {
      calls++;
      dispatched();
      await hold;
      assert.equal(signal.aborted, false);
      return { result: { published: true } };
    },
    checkpoint: {
      save: async () => {
        saved++;
      },
    },
  });
  const args = { agent_link: 'https://access.example/agent#fixture', document: 'concept:\n' };
  const write = backend.call('tonk_evaluate', args);
  await ready;
  const queued = backend.call('tonk_evaluate', args);
  const queuedFailure = assert.rejects(queued, /shutting down/);
  const closing = backend.close({ drain: false });
  finish();
  assert.deepEqual(await write, { published: true });
  await queuedFailure;
  await closing;
  assert.equal(calls, 1);
  assert.equal(saved, 1);
});

test('MCP discovers without OAuth and requires an agent link on every query/evaluation', async () => {
  const calls = [];
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async ({ input }) => {
      calls.push(input);
      return { result: { matches_after: [], commits: { claims: 0 } } };
    },
  });
  const handler = createTonkHTTP(async () => backend);
  const client = new Client({ name: 'agent-link-spike-test', version: '1' });
  await client.connect(
    new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
      fetch: (url, init) => handler.fetch(new Request(url, init)),
    }),
  );
  try {
    const tools = (await client.listTools()).tools;
    assert.deepEqual(tools.map(({ name }) => name).sort(), ['tonk_evaluate', 'tonk_query']);
    for (const tool of tools) {
      assert.ok(tool.inputSchema.required.includes('agent_link'));
      assert.equal(tool.inputSchema.properties.space, undefined);
      const args = {
        agent_link: 'https://tonk.example/agent#tonk-agent-v2=fixture',
        document: 'concept:\n',
      };
      const good = await client.callTool({ name: tool.name, arguments: args });
      assert.notEqual(good.isError, true);
      assert.ok(!JSON.stringify(good).includes(args.agent_link));
      assert.deepEqual(calls.at(-1), { tool: tool.name, ...args });
      const before = calls.length;
      const missing = await client.callTool({
        name: tool.name,
        arguments: { document: args.document },
      });
      assert.equal(missing.isError, true);
      assert.equal(calls.length, before);
      const override = await client.callTool({
        name: tool.name,
        arguments: { ...args, space: 'did:key:other' },
      });
      assert.equal(override.isError, true);
      assert.equal(calls.length, before);
    }
  } finally {
    await client.close();
    await handler.close();
  }
});

test('invalid shortcut origin is rejected before execution', async () => {
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: () => assert.fail('must not execute'),
  });
  await assert.rejects(
    backend.call('tonk_query', {
      agent_link: 'https://other.example/@/abc#secret',
      document: 'concept:\n',
    }),
    /Could not resolve/,
  );
});

test('shortcut failures never log upstream exception text', async (t) => {
  const logs = [];
  t.mock.method(console, 'error', (...args) => logs.push(args));
  t.mock.method(globalThis, 'fetch', async () => {
    throw Error('private-shortcut-fixture');
  });
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: () => assert.fail('must not execute'),
  });
  await assert.rejects(
    backend.call('tonk_query', {
      agent_link: 'https://access.example/@/fixture#private-shortcut-fixture',
      document: 'concept:\n',
    }),
    /Could not resolve/,
  );
  assert.deepEqual(logs, [['Agent link expansion failed.']]);
});

test('shortcut resolution keeps the seed out of HTTP and reattaches it for the worker', async (t) => {
  t.mock.method(globalThis, 'fetch', async (url, options) => {
    assert.equal(url.href, 'https://access.example/@/fixture');
    assert.equal(options.redirect, 'manual');
    return new Response(null, { status: 301, headers: { location: '/agent?agent=public-proof' } });
  });
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async ({ input }) => {
      assert.equal(
        input.agent_link,
        'https://access.example/agent?agent=public-proof#tonk-agent-v2=private-fixture',
      );
      return { result: {} };
    },
  });
  await backend.call('tonk_query', {
    agent_link: 'https://access.example/@/fixture#tonk-agent-v2=private-fixture',
    document: 'concept:\n',
  });
});

test('worker failure is not retried', async () => {
  let count = 0;
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async () => {
      count++;
      return {
        error: 'Applied locally; remote push could not be confirmed. Do not repeat the write.',
      };
    },
  });
  await assert.rejects(
    backend.call('tonk_evaluate', {
      agent_link: 'https://tonk.example/agent#fixture',
      document: 'concept:\n',
    }),
    /Do not repeat/,
  );
  assert.equal(count, 1);
});

test('bounded admission, serialization and shutdown prevent overlapping workers', async () => {
  let release;
  const hold = new Promise((resolve) => {
    release = resolve;
  });
  let calls = 0;
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    maxPending: 1,
    run: async () => {
      calls++;
      await hold;
      return { result: {} };
    },
  });
  const args = { agent_link: 'https://access.example/agent#fixture', document: 'concept:\n' };
  const first = backend.call('tonk_query', args);
  await assert.rejects(backend.call('tonk_query', args), /busy/);
  const closing = backend.close();
  await assert.rejects(backend.call('tonk_query', args), /unavailable/);
  release();
  await first;
  await closing;
  assert.equal(calls, 1);
});

test('checkpoint failure after an uncertain evaluation freezes queued calls without replay', async () => {
  let calls = 0,
    saves = 0;
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async () => {
      calls++;
      throw Error('lost child response');
    },
    checkpoint: {
      save: async () => {
        saves++;
        throw Error('storage unavailable');
      },
    },
  });
  const args = { agent_link: 'https://access.example/agent#fixture', document: 'concept:\n' };
  const first = backend.call('tonk_evaluate', args);
  const second = backend.call('tonk_evaluate', args);
  await assert.rejects(first, /Durable storage failed/);
  await assert.rejects(second, /recovery/);
  assert.equal(calls, 1);
  assert.equal(saves, 1);
});

test('shutdown aborts an active worker and checkpoints its closed state', async () => {
  let started;
  const ready = new Promise((resolve) => {
    started = resolve;
  });
  let closed = false,
    saved = false;
  const backend = agentLinkBackend({
    remote: 'https://access.example/ucan/',
    run: async ({ signal }) => {
      started();
      await new Promise((resolve) => signal.addEventListener('abort', resolve, { once: true }));
      closed = true;
      return { error: 'Unknown write outcome.' };
    },
    checkpoint: {
      save: async () => {
        assert.equal(closed, true);
        saved = true;
      },
    },
  });
  const call = backend.call('tonk_evaluate', {
    agent_link: 'https://access.example/agent#fixture',
    document: 'concept:\n',
  });
  await ready;
  const error = assert.rejects(call, /Unknown write outcome/);
  await backend.close({ cancel: true });
  await error;
  assert.equal(saved, true);
});
