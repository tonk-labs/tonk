import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { Readable } from 'node:stream';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash, randomBytes } from 'node:crypto';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { startWorker } from '../worker-process.mjs';
import { createHostedService } from '../hosted/service.mjs';
import { createDurableState } from '../hosted/durable-state.mjs';
assert.ok(process.env.TONK_WORKER_BINARY, 'TONK_WORKER_BINARY is required');
assert.ok(process.env.TONK_PYTHON, 'TONK_PYTHON with mcp==1.26.0 is required');
const issuer = 'https://fixture.example';
const redirect = 'http://127.0.0.1:54321/callback';
const clients = ['node', 'python'].map((clientId) => ({
  clientId,
  displayName: clientId + ' SDK',
  redirectUris: ['http://127.0.0.1/callback'],
  type: 'public',
  applicationType: 'native',
  enabled: true,
}));

test(
  'production hosted composition over real HTTP with Node/Python SDKs and native worker data',
  { timeout: 180000 },
  async () => {
    const directory = await mkdtemp(join(tmpdir(), 'tonk-hosted-sdk-'));
    const workers = [],
      spaces = [],
      wire = [];
    let service,
      server,
      base,
      credentials,
      version = 0,
      allocated = 0,
      checkpoints = 0;
    const storage = async (_url, options = {}) => {
      if (options.method !== 'PUT')
        return credentials
          ? new Response(credentials, { headers: { etag: String(version) } })
          : new Response(null, { status: 404 });
      assert.equal(
        options.headers[version ? 'if-match' : 'if-none-match'],
        version ? String(version) : '*',
      );
      credentials = options.body;
      version++;
      return new Response(null, { headers: { etag: String(version) } });
    };
    const options = {
      issuer,
      subject: 'fixture',
      provider: 'https://tonk.foundation',
      clientId: 'node',
      redirectUri: redirect,
      clients,
      dataRoot: directory,
      // Only account approval/membership and checkpoint storage are fixtures.
      // Tools dispatch through the production account backend into native workers.
      checkpoint: {
        async save() {
          checkpoints++;
        },
      },
      provision: async () => {
        const index = allocated++;
        assert.ok(index < workers.length);
        return {
          tenantId: String(index),
          deviceDid: 'did:key:fixture' + index,
          async authorize(payload) {
            assert.deepEqual(payload, { fixture: true });
            return { deviceDid: 'did:key:fixture' + index, rootDid: 'did:key:root' + index };
          },
        };
      },
      tenants: {
        async list(principal) {
          return [{ subject: spaces[Number(principal.tenantId)], name: 'Disposable' }];
        },
        async resolve(principal, space) {
          const i = Number(principal.tenantId);
          assert.equal(space, spaces[i], 'membership isolation');
          return workers[i];
        },
        async close() {},
      },
    };
    const request = (path, init = {}) => fetch(base + path, { redirect: 'manual', ...init });
    async function login(clientId) {
      const verifier = randomBytes(32).toString('base64url');
      const query = new URLSearchParams({
        client_id: clientId,
        redirect_uri: redirect,
        resource: issuer + '/mcp',
        scope: 'tonk',
        response_type: 'code',
        state: 'fixture',
        code_challenge_method: 'S256',
        code_challenge: createHash('sha256').update(verifier).digest('base64url'),
      });
      const first = await request('/oauth/authorize?' + query);
      assert.equal(first.status, 303);
      const cookie = first.headers.get('set-cookie').split(';')[0];
      const next = await request(
        new URL(first.headers.get('location')).pathname +
          new URL(first.headers.get('location')).search,
        { headers: { cookie } },
      );
      const approval = new URL(next.headers.get('location'));
      const callback = new URL(approval.searchParams.get('callback'));
      const result = await request('/oauth/callback', {
        method: 'POST',
        headers: { cookie, origin: issuer, 'content-type': 'application/json' },
        body: JSON.stringify({
          request: callback.searchParams.get('request'),
          authorize: Buffer.from(JSON.stringify({ fixture: true })).toString('base64'),
        }),
      });
      const code = new URL((await result.json()).redirect).searchParams.get('code');
      const token = await request('/oauth/token', {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({
          client_id: clientId,
          grant_type: 'authorization_code',
          code,
          code_verifier: verifier,
          redirect_uri: redirect,
          resource: issuer + '/mcp',
        }),
      });
      assert.equal(token.status, 200);
      return token.json();
    }
    async function connect(token, modern = false) {
      const client = new Client(
        { name: 'hosted-native-integration', version: '1' },
        modern ? { versionNegotiation: { mode: { pin: '2026-07-28' } } } : {},
      );
      await client.connect(
        new StreamableHTTPClientTransport(new URL(base + '/mcp'), {
          requestInit: { headers: { authorization: 'Bearer ' + token } },
        }),
      );
      return client;
    }
    try {
      for (let i = 0; i < 2; i++) {
        const worker = await startWorker({
          binary: process.env.TONK_WORKER_BINARY,
          dataDirectory: join(directory, 'worker-' + i),
        });
        workers.push(worker);
        const created = await (
          await worker.request('/api/repository/sdk-fixture-' + i, {
            method: 'PUT',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ branch: { main: {} } }),
          })
        ).json();
        spaces.push(created.name.startsWith('did:') ? created.name : 'did:key:' + created.name);
      }
      service = await createHostedService({
        ...options,
        durable: await createDurableState({ fetch: storage }),
      });
      server = createServer(async (req, res) => {
        try {
          const chunks = [];
          for await (const chunk of req) chunks.push(chunk);
          const abort = new AbortController();
          res.on('close', () => abort.abort());
          const response = await service.fetch(
            new Request(issuer + req.url, {
              method: req.method,
              headers: req.headers,
              signal: abort.signal,
              ...(['GET', 'HEAD'].includes(req.method) ? {} : { body: Buffer.concat(chunks) }),
            }),
          );
          wire.push(response.headers.get('content-type'));
          res.writeHead(response.status, Object.fromEntries(response.headers));
          if (response.body) Readable.fromWeb(response.body).pipe(res);
          else res.end();
        } catch {
          res.writeHead(503).end();
        }
      });
      server.listen(0, '127.0.0.1');
      await once(server, 'listening');
      base = 'http://127.0.0.1:' + server.address().port;
      const a = await login('node'),
        b = await login('python');
      assert.equal(checkpoints, 2);
      const document = await readFile(
        new URL('../fixtures/authoring/tasks.notation', import.meta.url),
        'utf8',
      );
      for (const [index, token] of [a, b].entries()) {
        const client = await connect(token.access_token, index === 1);
        try {
          assert.equal((await client.listTools()).tools.length, 4);
          const list = await client.callTool({ name: 'tonk_list_spaces', arguments: {} });
          assert.equal(list.structuredContent.spaces[0].subject, spaces[index]);
          const result = await client.callTool({
            name: 'tonk_evaluate',
            arguments: { space: spaces[index], document },
          });
          assert.notEqual(result.isError, true, JSON.stringify(result.content));
          const read = await client.callTool({
            name: 'tonk_query',
            arguments: { space: spaces[index], document: 'integration-task:\n  title: ?title\n' },
          });
          assert.ok(JSON.stringify(read.structuredContent).includes('First task'));
          const denied = await client.callTool({
            name: 'tonk_query',
            arguments: { space: spaces[1 - index], document: 'integration-task:\n' },
          });
          assert.equal(denied.isError, true);
        } finally {
          await client.close();
        }
      }
      const python = spawn(
        process.env.TONK_PYTHON,
        [new URL('./python-client.py', import.meta.url).pathname],
        { stdio: ['pipe', 'pipe', 'pipe'] },
      );
      let output = '',
        errors = '';
      python.stdout.on('data', (data) => (output += data));
      python.stderr.on('data', (data) => (errors += data));
      python.stdin.end(
        JSON.stringify({ url: base + '/mcp', token: b.access_token, space: spaces[1] }),
      );
      assert.equal((await once(python, 'exit'))[0], 0, errors);
      assert.match(output, /readback passed/);
      await service.close();
      service = await createHostedService({
        ...options,
        durable: await createDurableState({ fetch: storage }),
      });
      const refresh = await request('/oauth/token', {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({
          client_id: 'python',
          grant_type: 'refresh_token',
          refresh_token: b.refresh_token,
        }),
      });
      assert.equal(refresh.status, 200);
      const renewed = await refresh.json(),
        reconnected = await connect(renewed.access_token);
      try {
        const read = await reconnected.callTool({
          name: 'tonk_query',
          arguments: {
            space: spaces[1],
            document: 'integration-task:\n  this: urn:test:python\n  title: ?title\n',
          },
        });
        assert.ok(JSON.stringify(read.structuredContent).includes('Python SDK'));
      } finally {
        await reconnected.close();
      }
      assert.ok(wire.some((type) => type?.includes('text/event-stream')));
      assert.ok(wire.some((type) => type?.includes('application/json')));
    } finally {
      await service?.close();
      server?.closeAllConnections();
      await new Promise((resolve) => (server ? server.close(resolve) : resolve()));
      await Promise.allSettled(workers.map((worker) => worker.close()));
      await rm(directory, { recursive: true, force: true });
    }
  },
);
