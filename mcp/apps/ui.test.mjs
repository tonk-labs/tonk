import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { accountBackend } from '../account-backend.mjs';
import { createTonkHTTP } from '../http.mjs';
import { createWorkerUI, parseViewRoute, renderWorkerUI } from './ui.mjs';

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
    const tool = (await connection.client.listTools()).tools.find(
      (t) => t.name === 'tonk_open_space',
    );
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
      'tonk_show_view',
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

test('a view route reads as tonk_outline writes it', () => {
  assert.deepEqual(parseViewRoute('todo'), { model: 'todo' });
  assert.deepEqual(parseViewRoute('urn:trip:lisbon@trip'), {
    model: 'trip',
    entity: 'urn:trip:lisbon',
  });
  assert.deepEqual(parseViewRoute('did:key:zABC@person!card'), {
    model: 'person',
    entity: 'did:key:zABC',
    facet: 'card',
  });
  for (const route of ['', '@todo', 'x@', 'todo!', 'a<b', 'x'.repeat(2049)])
    assert.throws(() => parseViewRoute(route), route);
});

test('showing a view opens a session for its space and mounts that display alone', async () => {
  const opened = [];
  const backend = {
    ...accountBackend({
      principal: { tenantId: 'tenant-abcdef' },
      tenants: { list: async () => [], resolve: async () => ({}) },
    }),
    openSpace: async (space) => {
      opened.push(space);
      return { token: 'private-browser-token' };
    },
  };
  const connection = await connect(backend, {
    registerUI: createWorkerUI({ origin: 'https://mcp.example.test' }),
  });
  try {
    const shown = await connection.client.callTool({
      name: 'tonk_show_view',
      arguments: { space: 'did:key:one', route: 'urn:trip:lisbon@trip' },
    });
    assert.deepEqual(shown.structuredContent, {
      subject: 'did:key:one',
      path: '/',
      view: { model: 'trip', entity: 'urn:trip:lisbon' },
    });
    assert.equal(shown._meta.session.token, 'private-browser-token');
    assert.ok(!JSON.stringify([shown.content, shown.structuredContent]).includes('private-'));

    const refused = await connection.client.callTool({
      name: 'tonk_show_view',
      arguments: { space: 'did:key:one', route: '@trip' },
    });
    assert.equal(refused.isError, true);
    assert.deepEqual(opened, ['did:key:one'], 'a bad route opens no session');
  } finally {
    await connection.close();
  }
});
