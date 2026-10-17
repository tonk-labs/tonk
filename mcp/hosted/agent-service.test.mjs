import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createAgentService } from './agent-service.mjs';

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
