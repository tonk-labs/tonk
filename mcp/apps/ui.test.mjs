import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { accountBackend } from '../account-backend.mjs';
import { createTonkHTTP } from '../http.mjs';
import { createWorkerUI, renderWorkerUI } from './ui.mjs';

async function connect(backend, options) {
  const handler = createTonkHTTP(async () => backend, options);
  const client = new Client({ name: 'general-host', version: '1' });
  await client.connect(
    new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
      fetch: (url, init) => handler.fetch(new Request(url, init)),
    }),
  );
  return {
    client,
    async close() {
      await client.close();
      await handler.close();
    },
  };
}

test('ChatGPT origin config controls both embedded requests and CSP, and errors hide secrets', async () => {
  const origin = 'https://mcp.example.test';
  const backend = {
    capabilities: [],
    call() {},
    openSpace() {
      throw Error('private-session-secret');
    },
  };
  const connection = await connect(backend, { registerUI: createWorkerUI({ origin }) });
  try {
    const tool = (await connection.client.listTools()).tools[0];
    const resource = (await connection.client.readResource({ uri: tool._meta.ui.resourceUri }))
      .contents[0];
    assert.ok(resource.text.includes(origin));
    assert.ok(!resource.text.includes('tonk-mcp-test.tonk.workers.dev'));
    assert.ok(!resource.text.includes('__TONK_MCP_ORIGIN__'));
    assert.deepEqual(resource._meta.ui.csp.connectDomains, [origin]);
    assert.deepEqual(resource._meta.ui.csp.frameDomains, [origin]);
    const failed = await connection.client.callTool({ name: 'tonk_open_space', arguments: {} });
    assert.equal(failed.isError, true);
    assert.ok(!JSON.stringify(failed).includes('private-session-secret'));
  } finally {
    await connection.close();
  }
  for (const origin of [
    'https://example.test/path',
    'http://example.test',
    'https://user:password@example.test',
  ]) {
    assert.throws(() => renderWorkerUI({ origin }));
  }
});

test('account UI requires a selected space and keeps browser credentials in private metadata', async () => {
  const calls = [];
  const principal = { tenantId: 'tenant-abcdef' };
  const tenants = {
    list: async () => [{ subject: 'did:key:one', name: 'One' }],
    resolve: async (actual, space) => {
      assert.equal(actual, principal);
      assert.equal(space, 'did:key:one');
      calls.push(space);
      return { request: async () => Response.json({}) };
    },
  };
  const backend = {
    ...accountBackend({ principal, tenants }),
    openSpace: async (space) => {
      await tenants.resolve(principal, space);
      return { token: 'private-browser-token', resumeToken: 'private-resume-token' };
    },
  };
  const connection = await connect(backend, {
    registerUI: createWorkerUI({ origin: 'https://mcp.example.test' }),
  });
  try {
    const tools = (await connection.client.listTools()).tools;
    assert.deepEqual(tools.map((tool) => tool.name).sort(), [
      'tonk_evaluate',
      'tonk_guide',
      'tonk_list_spaces',
      'tonk_open_space',
      'tonk_query',
    ]);
    assert.ok(
      tools.find((tool) => tool.name === 'tonk_open_space').inputSchema.required.includes('space'),
    );
    const missing = await connection.client.callTool({ name: 'tonk_open_space', arguments: {} });
    assert.equal(missing.isError, true);
    assert.equal(calls.length, 0);
    const opened = await connection.client.callTool({
      name: 'tonk_open_space',
      arguments: { space: 'did:key:one', path: '/' },
    });
    assert.equal(opened.structuredContent.subject, 'did:key:one');
    assert.equal(opened._meta.session.token, 'private-browser-token');
    assert.ok(!JSON.stringify([opened.content, opened.structuredContent]).includes('private-'));
    assert.deepEqual(calls, ['did:key:one']);
  } finally {
    await connection.close();
  }
});
