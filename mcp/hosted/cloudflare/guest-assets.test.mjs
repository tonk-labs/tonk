import { test } from 'node:test';
import assert from 'node:assert/strict';
import { guestAsset, guestAssetPaths } from './guest-assets.mjs';

const assets = { fetch: async () => new Response('<!doctype html>') };

test('guest assets cannot use WebAuthn, so they cannot claim the apex RP ID', async () => {
  for (const path of guestAssetPaths) {
    const response = await guestAsset(new Request('https://agent-dev.tonk.host' + path), assets);
    assert.equal(response.status, 200);
    assert.equal(
      response.headers.get('Permissions-Policy'),
      'publickey-credentials-get=(), publickey-credentials-create=()',
    );
    assert.equal(response.headers.get('X-Content-Type-Options'), 'nosniff');
  }
});
