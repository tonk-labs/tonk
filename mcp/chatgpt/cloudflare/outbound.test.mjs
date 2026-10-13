import { test } from 'node:test';
import assert from 'node:assert/strict';
import { outbound } from './outbound.mjs';

test('egress permits only the development service and does not follow redirects', async () => {
  let calls = 0;
  const forward = async (request) => {
    calls++;
    assert.equal(request.redirect, 'manual');
    return new Response(null, { status: 302, headers: { location: 'http://127.0.0.1/' } });
  };
  const result = await outbound(
    new Request('https://tonk.foundation/object/example'),
    {},
    {},
    forward,
  );
  assert.equal(result.status, 302);
  for (const url of [
    'http://tonk.foundation/',
    'https://tonk.network/',
    'http://127.0.0.1/',
    'https://tonk.foundation.attacker.test/',
    'https://tonk.foundation:8443/',
    'http://checkpoint.internal/wrong',
  ]) {
    assert.equal((await outbound(new Request(url), {}, {}, forward)).status, 403);
  }
  assert.equal(calls, 1);
});

test('private checkpoint proxy namespaces the instance and requires conditional writes', async () => {
  const env = {
    CHECKPOINTS: {
      get: async (key) => {
        assert.equal(key, 'checkpoint/instance/state-v1.json');
        return null;
      },
      put: async (key, body, options) => {
        assert.equal(key, 'checkpoint/instance/state-v1.json');
        assert.equal(new TextDecoder().decode(body), 'state');
        assert.equal(options.onlyIf.get('if-none-match'), '*');
        return { httpEtag: '"first"' };
      },
    },
  };
  const context = { containerId: 'instance' },
    url = 'http://checkpoint.internal/state';
  assert.equal((await outbound(new Request(url), env, context)).status, 404);
  assert.equal(
    (await outbound(new Request(url, { method: 'PUT', body: 'state' }), env, context)).status,
    428,
  );
  const result = await outbound(
    new Request(url, { method: 'PUT', body: 'state', headers: { 'if-none-match': '*' } }),
    env,
    context,
  );
  assert.equal(result.headers.get('etag'), '"first"');
});

test('credential state uses a separate private conditional object', async () => {
  const result = await outbound(
    new Request('http://checkpoint.internal/credentials', {
      method: 'PUT',
      headers: { 'if-none-match': '*' },
      body: '{}',
    }),
    {
      CHECKPOINTS: {
        put: async (key) => {
          assert.equal(key, 'checkpoint/instance/credentials-v1.json');
          return { httpEtag: '"saved"' };
        },
      },
    },
    { containerId: 'instance' },
  );
  assert.equal(result.status, 200);
});
