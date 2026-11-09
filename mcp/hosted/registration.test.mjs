import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRegistrations } from './registration.mjs';
const metadata = {
  client_name: 'SDK',
  redirect_uris: ['https://client.example/callback'],
  token_endpoint_auth_method: 'none',
};
test('enrollment quota and rate survive restart; expiration frees bounded capacity', async () => {
  let time = 100000;
  const options = { now: () => time, capacity: 9, lifetime: 120000 };
  const registry = createRegistrations(options);
  const results = await Promise.all(
    Array.from({ length: 10 }, async () => registry.register(metadata)),
  );
  assert.equal(results.filter((r) => r.client).length, 8);
  assert.equal(results.filter((r) => r.status === 429).length, 2);
  const restored = createRegistrations({ ...options, saved: registry.snapshot() });
  assert.equal(restored.register(metadata).status, 429);
  time += 60001;
  assert.ok(restored.register(metadata).client);
  assert.equal(restored.register(metadata).status, 429);
  time += 120000;
  assert.ok(restored.register(metadata).client);
});
test('registration rejects secret auth, unsupported metadata, and unsafe redirects', () => {
  const registry = createRegistrations();
  for (const patch of [
    { token_endpoint_auth_method: 'client_secret_basic' },
    { client_name: '' },
    { redirect_uris: ['http://192.168.0.1/callback'] },
    { redirect_uris: ['http://localhost/callback'], application_type: 'native' },
    { redirect_uris: ['https://client.example/callback#secret'] },
    { redirect_uris: ['https://user:secret@client.example/callback'] },
    { client_uri: 'http://169.254.169.254/' },
    { scope: 'admin' },
    { grant_types: ['client_credentials'] },
    { response_types: ['token'] },
  ])
    assert.equal(registry.register({ ...metadata, ...patch }).status, 400);
  assert.equal(registry.snapshot().entries.length, 0);
});
