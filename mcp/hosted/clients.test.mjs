import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createClientRegistry } from './clients.mjs';
const native = {
  clientId: 'native',
  displayName: 'Native',
  type: 'public',
  applicationType: 'native',
  enabled: true,
  redirectUris: ['http://127.0.0.1/callback?mode=one', 'http://[::1]/callback'],
};
test('native port exception preserves literal host, path and query', () => {
  const registry = createClientRegistry([native]);
  assert.ok(registry.accepts('native', 'http://127.0.0.1:54321/callback?mode=one'));
  assert.ok(registry.accepts('native', 'http://[::1]:54321/callback'));
  for (const uri of [
    'http://localhost/callback',
    'http://127.1/callback?mode=one',
    'http://2130706433/callback?mode=one',
    'http://127.0.0.1.evil.test/callback?mode=one',
    'http://127.0.0.1/callback?mode=two',
    'http://127.0.0.1/other?mode=one',
    'http://192.168.1.1/callback',
    'http://user@127.0.0.1/callback',
    'http://[::1]/callback#fragment',
  ])
    assert.equal(registry.accepts('native', uri), false, uri);
});
test('registry validates bounds, duplicates, client type and web redirects', () => {
  for (const entries of [
    [],
    Array(65).fill(native),
    [native, native],
    [{ ...native, type: 'confidential' }],
    [{ ...native, applicationType: 'web' }],
    [{ ...native, redirectUris: ['https://a.test/callback#hash'] }],
  ])
    assert.throws(() => createClientRegistry(entries));
  const registry = createClientRegistry([
    { ...native, applicationType: 'web', redirectUris: ['https://a.test/callback'] },
  ]);
  assert.ok(registry.accepts('native', 'https://a.test/callback'));
  assert.equal(registry.accepts('native', 'https://a.test:444/callback'), false);
});

test('remote callbacks reject IP/private-name forms and even empty fragments', () => {
  for (const uri of [
    'https://127.0.0.1/callback',
    'https://192.168.1.1/callback',
    'https://169.254.169.254/callback',
    'https://[::1]/callback',
    'https://[fd00::1]/callback',
    'https://localhost/callback',
    'https://host.local/callback',
    'https://host.internal/callback',
    'https://host.home.arpa/callback',
    'https://client.example/callback#',
  ]) {
    assert.throws(
      () => createClientRegistry([{ ...native, applicationType: 'web', redirectUris: [uri] }]),
      /Invalid redirect/,
    );
  }
});
