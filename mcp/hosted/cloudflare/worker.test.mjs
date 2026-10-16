// Run with --experimental-test-module-mocks: the Containers runtime only loads
// inside workerd, and these checks exercise the edge routing alone.
import { test, mock } from 'node:test';
import assert from 'node:assert/strict';

mock.module('@cloudflare/containers', {
  namedExports: { Container: class {}, ContainerProxy: class {} },
});
const { default: worker } = await import('./worker.mjs');

const env = {
  TONK_MCP_ISSUER: 'https://agent-dev.tonk.host',
  TONK_PROVIDER_ORIGIN: 'https://tonk.foundation',
  ASSETS: { fetch: async () => new Response('<!doctype html>') },
};

test('guest assets cannot use WebAuthn, so they cannot claim the apex RP ID', async () => {
  for (const path of ['/worker-guest.html', '/space-runtime.json']) {
    const response = await worker.fetch(new Request(env.TONK_MCP_ISSUER + path), env);
    assert.equal(response.status, 200);
    assert.equal(
      response.headers.get('Permissions-Policy'),
      'publickey-credentials-get=(), publickey-credentials-create=()',
    );
  }
});

test('only the issuer origin is answered', async () => {
  const response = await worker.fetch(new Request('https://tonk.host/worker-guest.html'), env);
  assert.equal(response.status, 421);
});
