import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { agentLinkBackend } from './agent-link-backend.mjs';
import { createTonkHTTP } from './http.mjs';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

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
