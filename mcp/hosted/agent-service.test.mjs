import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createAgentService } from './agent-service.mjs';
import { createServer, request as httpRequest } from 'node:http';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { invoke } from '../agent-link-backend.mjs';

test('a disconnected HTTP socket leaves its dispatched subprocess running through checkpoint', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-hosted-disconnect-'));
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
  let dispatched, checkpointed, socketClosed;
  const ready = new Promise((resolve) => {
    dispatched = resolve;
  });
  const saved = new Promise((resolve) => {
    checkpointed = resolve;
  });
  const disconnected = new Promise((resolve) => {
    socketClosed = resolve;
  });
  let calls = 0,
    published = false,
    workerSignal;
  const service = await createAgentService({
    issuer: 'https://mcp.example',
    provider: 'https://tonk.example',
    binary,
    directory,
    run: (options) => {
      calls++;
      workerSignal = options.signal;
      const result = invoke(options).then((result) => {
        published = result.result?.published === true;
        return result;
      });
      dispatched();
      return result;
    },
    checkpoint: {
      save: async () => {
        checkpointed();
      },
    },
  });
  // Exercise the server.mjs response-close cancellation boundary through a real
  // socket, with the hosted service and actual worker subprocess behind it.
  const server = createServer(async (req, res) => {
    const abort = new AbortController();
    res.on('close', () => {
      abort.abort();
      socketClosed();
    });
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const response = await service.fetch(
      new Request('https://mcp.example/mcp', {
        method: req.method,
        headers: req.headers,
        body: Buffer.concat(chunks),
        signal: abort.signal,
      }),
    );
    if (!res.destroyed) {
      res.writeHead(response.status, Object.fromEntries(response.headers));
      res.end(await response.text());
    } else await response.body?.cancel();
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(async () => {
    await service.close();
    await new Promise((resolve) => server.close(resolve));
  });
  const request = httpRequest({
    hostname: '127.0.0.1',
    port: server.address().port,
    path: '/mcp',
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': '2025-03-26',
    },
  });
  request.on('error', () => {}); // Client intentionally closes before receiving a response.
  request.end(
    JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: {
        name: 'tonk_evaluate',
        arguments: { agent_link: 'https://tonk.example/agent#fixture', document: 'concept:\n' },
      },
    }),
  );
  await ready;
  request.destroy();
  await disconnected;
  assert.equal(workerSignal.aborted, false);
  await saved;
  assert.equal(published, true);
  assert.equal(calls, 1);
});

test('hosted shutdown drains the active write through checkpoint without cancelling it', async () => {
  let dispatched, finish;
  const ready = new Promise((resolve) => {
    dispatched = resolve;
  });
  const hold = new Promise((resolve) => {
    finish = resolve;
  });
  let workerSignal,
    saved = false;
  const service = await createAgentService({
    issuer: 'https://mcp.example',
    provider: 'https://tonk.example',
    directory: '/unused',
    checkpoint: {
      save: async () => {
        saved = true;
      },
    },
    run: async ({ signal }) => {
      workerSignal = signal;
      dispatched();
      await hold;
      return { result: { matches_after: [] } };
    },
  });
  const response = service.fetch(
    new Request('https://mcp.example/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        'mcp-protocol-version': '2025-03-26',
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: {
          name: 'tonk_evaluate',
          arguments: {
            agent_link: 'https://tonk.example/agent#fixture',
            document: 'concept:\n',
          },
        },
      }),
    }),
  );
  await ready;
  const closing = service.close();
  const wasAborted = workerSignal.aborted;
  finish();
  await response;
  await closing;
  assert.equal(wasAborted, false);
  assert.equal(saved, true);
});

test('hosted URL discovers without OAuth, checkpoints calls, and rejects old auth endpoints', async () => {
  let writes = 0,
    saved = 0;
  const service = await createAgentService({
    issuer: 'https://mcp.example',
    provider: 'https://tonk.example',
    directory: '/unused-test-root',
    checkpoint: {
      save: async () => {
        saved++;
      },
    },
    run: async () => {
      writes++;
      return { result: { matches_after: [] } };
    },
  });
  for (const [url, init, expected] of [
    ['https://other.example/mcp', {}, 421],
    ['https://mcp.example/oauth/authorize', {}, 404],
    [
      'https://mcp.example/mcp',
      { method: 'POST', headers: { origin: 'https://evil.example' } },
      403,
    ],
    ['https://mcp.example/mcp', {}, 405],
  ])
    assert.equal((await service.fetch(new Request(url, init))).status, expected);
  const client = new Client({ name: 'hosted-agent-test', version: '1' });
  try {
    await client.connect(
      new StreamableHTTPClientTransport(new URL('https://mcp.example/mcp'), {
        fetch: (url, init) => service.fetch(new Request(url, init)),
      }),
    );
    assert.equal((await client.listTools()).tools.length, 2);
    const result = await client.callTool({
      name: 'tonk_evaluate',
      arguments: {
        agent_link: 'https://tonk.example/agent#fixture',
        document: 'concept:\n',
      },
    });
    assert.notEqual(result.isError, true);
    assert.equal(writes, 1);
    assert.equal(saved, 1);
  } finally {
    await client.close();
    await service.close();
  }
});

test('legacy stateless HTTP discovers link schemas without account sessions', async () => {
  const service = await createAgentService({
    issuer: 'https://mcp.example',
    provider: 'https://tonk.example',
    directory: '/unused',
    checkpoint: { save: async () => {} },
    run: () => assert.fail('no execution'),
  });
  const request = (body) =>
    service.fetch(
      new Request('https://mcp.example/mcp', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'mcp-protocol-version': '2025-03-26',
        },
        body: JSON.stringify(body),
      }),
    );
  try {
    const response = await request({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} });
    assert.equal(response.status, 200);
    const text = await response.text();
    assert.ok(text.includes('agent_link'));
    assert.equal(response.headers.get('www-authenticate'), null);
    assert.equal(response.headers.get('mcp-session-id'), null);
  } finally {
    await service.close();
  }
});
