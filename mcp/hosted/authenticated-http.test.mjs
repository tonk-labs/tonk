import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';
import { createWorkerUI } from '../apps/ui.mjs';
import { createTonkHTTP } from '../http.mjs';
const url = 'https://mcp.example/mcp';
const apps = {
  extensions: { 'io.modelcontextprotocol/ui': { mimeTypes: ['text/html;profile=mcp-app'] } },
};
const backend = {
  capabilities: ['tonk_list_spaces', 'tonk_query', 'tonk_evaluate', 'tonk_guide'],
  call: async () => ({ ok: true }),
  openSpace: async () => ({ token: 'private-browser-token', resumeToken: 'private-resume-token' }),
};
function fixture(options = {}) {
  return createAuthenticatedTonkHTTP({
    oauth: {
      authenticate: (request) => {
        const clientId = request.headers.get('authorization');
        return clientId
          ? { clientId, tenantId: clientId, rootDid: clientId, deviceDid: clientId }
          : undefined;
      },
      challenge: () => new Response(null, { status: 401 }),
      close() {},
    },
    resolveBackend: async () => backend,
    registerUI: createWorkerUI({ origin: 'https://mcp.example' }),
    ...options,
  });
}
async function connect(app, capabilities = {}, identity = 'a', extra = {}) {
  const client = new Client(
    { name: 'untrusted-client-name', version: '1' },
    { capabilities, ...extra },
  );
  const transport = new StreamableHTTPClientTransport(new URL(url), {
    fetch: (url, init) =>
      app.fetch(
        new Request(url, {
          ...init,
          headers: { ...Object.fromEntries(new Headers(init?.headers)), authorization: identity },
        }),
      ),
  });
  await client.connect(transport);
  return { client, transport };
}

test('SDK stateless legacy factory loses initialize capabilities on the next POST', async () => {
  const seen = [];
  const app = createTonkHTTP(async () => backend, {
    registerUI: (server) => seen.push(server.server.getClientCapabilities()),
  });
  const { client } = await connect(app, apps);
  await client.listTools();
  assert.ok(seen.length >= 2);
  assert.ok(seen.every((value) => value === undefined));
  await client.close();
  await app.close();
});

test('headless catalogs exclude Apps; SDK session retains negotiated UI across POSTs and isolates accounts', async () => {
  const app = fixture();
  const headless = await connect(app),
    ui = await connect(app, apps);
  try {
    assert.deepEqual((await headless.client.listTools()).tools.map((t) => t.name).sort(), [
      'tonk_evaluate',
      'tonk_guide',
      'tonk_list_spaces',
      'tonk_query',
    ]);
    assert.equal(headless.transport.sessionId, undefined);
    const tools = (await ui.client.listTools()).tools;
    assert.equal(tools.length, 5);
    assert.ok(ui.transport.sessionId);
    const opened = await ui.client.callTool({ name: 'tonk_open_space', arguments: { path: '/' } });
    assert.equal(opened._meta.session.token, 'private-browser-token');
    assert.ok(
      !JSON.stringify({
        content: opened.content,
        structuredContent: opened.structuredContent,
      }).includes('private-'),
    );
    assert.ok(
      !(await headless.client.listResources()).resources.some((r) => r.uri.startsWith('ui:')),
    );
    const response = await app.fetch(
      new Request(url, {
        method: 'POST',
        headers: {
          authorization: 'b',
          'mcp-session-id': ui.transport.sessionId,
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
        },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
      }),
    );
    assert.equal(response.status, 404);
    assert.equal((await ui.client.listTools()).tools.length, 5);
    await assert.rejects(
      headless.client.callTool({ name: 'tonk_open_space', arguments: {} }),
      /not found/,
    );
  } finally {
    await headless.client.close();
    await ui.client.close();
    await app.close();
  }
});

test('session capacity, expiry, unsupported methods, hostile origins and body limits fail closed', async () => {
  let now = 0;
  const app = fixture({ now: () => now, sessionCapacity: 1, sessionLifetime: 100 });
  const ui = await connect(app, apps);
  try {
    await assert.rejects(connect(app, apps, 'b'), /Error POSTing/);
    const response = await app.fetch(
      new Request(url, { method: 'GET', headers: { authorization: 'a' } }),
    );
    assert.equal(response.status, 405);
    assert.equal(response.headers.get('allow'), 'POST');
    assert.equal(
      (
        await app.fetch(
          new Request(url, {
            method: 'POST',
            headers: { origin: 'https://evil.test', authorization: 'a' },
          }),
        )
      ).status,
      403,
    );
    assert.equal(
      (
        await app.fetch(
          new Request(url, { method: 'POST', headers: { authorization: 'a' }, body: '{' }),
        )
      ).status,
      400,
    );
    assert.equal(
      (
        await app.fetch(
          new Request(url, {
            method: 'POST',
            headers: { authorization: 'a' },
            body: 'x'.repeat(200001),
          }),
        )
      ).status,
      413,
    );
    now = 101;
    // The transport can reinitialize after 404. Directly inspect the expired ID.
    assert.equal(
      (
        await app.fetch(
          new Request(url, {
            method: 'POST',
            headers: { authorization: 'a', 'mcp-session-id': ui.transport.sessionId },
            body: '{}',
          }),
        )
      ).status,
      404,
    );
  } finally {
    await ui.client.close();
    await app.close();
  }
});

test('modern SDK negotiates 2026-07-28 and carries UI capability independently on each request', async () => {
  const app = fixture();
  const ui = await connect(app, apps, 'a', { versionNegotiation: { mode: { pin: '2026-07-28' } } });
  const headless = await connect(app, {}, 'a', {
    versionNegotiation: { mode: { pin: '2026-07-28' } },
  });
  try {
    assert.equal(ui.transport.sessionId, undefined);
    assert.equal((await ui.client.listTools()).tools.length, 5);
    assert.equal((await headless.client.listTools()).tools.length, 4);
    assert.equal((await ui.client.listTools()).tools.length, 5);
  } finally {
    await ui.client.close();
    await headless.client.close();
    await app.close();
  }
});

test('oversized evaluate response fails once without replaying the write', async () => {
  let writes = 0;
  const app = fixture({
    resolveBackend: async () => ({
      capabilities: ['tonk_evaluate'],
      call: async () => {
        writes++;
        return { data: 'x'.repeat(4 * 1024 * 1024) };
      },
    }),
  });
  const { client } = await connect(app);
  try {
    await assert.rejects(
      client.callTool({ name: 'tonk_evaluate', arguments: { document: 'book:\n' } }),
    );
    assert.equal(writes, 1);
  } finally {
    await client.close();
    await app.close();
  }
});

test(
  'aborted HTTP write reaches backend cancellation and is never replayed',
  { timeout: 3000 },
  async () => {
    let enter,
      writes = 0,
      cancelled = false;
    const entered = new Promise((resolve) => (enter = resolve));
    const app = fixture({
      resolveBackend: async () => ({
        capabilities: ['tonk_evaluate'],
        async call(_name, _args, signal) {
          writes++;
          enter();
          await new Promise((resolve) => {
            signal.addEventListener(
              'abort',
              () => {
                cancelled = true;
                resolve();
              },
              { once: true },
            );
          });
          throw Error('uncertain write');
        },
      }),
    });
    const abort = new AbortController();
    const pending = app.fetch(
      new Request(url, {
        method: 'POST',
        signal: abort.signal,
        headers: {
          authorization: 'a',
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/call',
          params: { name: 'tonk_evaluate', arguments: { document: 'book:\n' } },
        }),
      }),
    );
    try {
      await entered;
      abort.abort();
      await pending;
      assert.equal(cancelled, true);
      assert.equal(writes, 1);
    } finally {
      await app.close();
    }
  },
);
