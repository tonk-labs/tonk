import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { accountBackend } from './account-backend.mjs';
import { createTonkHTTP } from './http.mjs';

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

test('account HTTP works without UI and revalidates every selected space', async () => {
  const principal = { tenantId: 'test-account' };
  const calls = [];
  let allowed = true;
  const tenants = {
    async list(received) {
      assert.equal(received, principal);
      return [{ subject: 'did:key:one' }];
    },
    async resolve(received, subject) {
      assert.equal(received, principal);
      assert.equal(subject, 'did:key:one');
      if (!allowed) throw Error('private authority details');
      return {
        request: async (path, options) => {
          calls.push({ path, ...options });
          return Response.json({ revision: { opaque: 1 }, matches: [] });
        },
      };
    },
  };
  const connection = await connect(accountBackend({ principal, tenants }));
  try {
    const { client } = connection;
    const tools = (await client.listTools()).tools;
    assert.deepEqual(tools.map((tool) => tool.name).sort(), [
      'tonk_evaluate',
      'tonk_guide',
      'tonk_list_spaces',
      'tonk_query',
    ]);
    assert.ok(tools.every((tool) => !tool._meta));
    assert.equal((await client.callTool({ name: 'tonk_guide', arguments: {} })).isError, undefined);
    const spaces = await client.callTool({ name: 'tonk_list_spaces', arguments: {} });
    assert.deepEqual(spaces.structuredContent.spaces, [{ subject: 'did:key:one' }]);
    const arguments_ = { space: 'did:key:one', document: 'example:' };
    assert.equal(
      (await client.callTool({ name: 'tonk_query', arguments: arguments_ })).isError,
      undefined,
    );
    assert.equal(calls.length, 1);
    assert.ok(calls[0].path.includes('did%3Akey%3Aone/branch/main/evaluate?transact=false'));
    allowed = false;
    const denied = await client.callTool({ name: 'tonk_evaluate', arguments: arguments_ });
    assert.equal(denied.isError, true);
    assert.ok(!JSON.stringify(denied).includes('private authority'));
    assert.equal(calls.length, 1);
  } finally {
    await connection.close();
  }
});

test('read-only account capability cannot dispatch evaluation directly or through MCP', async () => {
  const backend = accountBackend({
    principal: {},
    allowsWrites: false,
    tenants: {
      resolve() {
        throw Error('must not resolve a worker');
      },
    },
  });
  await assert.rejects(
    backend.call('tonk_evaluate', { space: 'did:key:one', document: 'example!:' }),
    /unavailable/,
  );
  const connection = await connect(backend);
  try {
    assert.ok(
      !(await connection.client.listTools()).tools.some((tool) => tool.name === 'tonk_evaluate'),
    );
    await assert.rejects(
      connection.client.callTool({ name: 'tonk_evaluate', arguments: {} }),
      /not found/,
    );
  } finally {
    await connection.close();
  }
});
