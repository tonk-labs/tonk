import { test } from 'node:test';
import assert from 'node:assert/strict';
import { discovery } from './discovery.mjs';
import { createTonkOAuth } from '../oauth.mjs';

test('registration discovery agrees with the live issuer and never starts authorization', async () => {
  const issuer = 'https://tonk-mcp.example';
  const oauth = createTonkOAuth({ issuer, clientId: 'test', redirectUris: ['https://client.example/callback'],
    provisionAccount: () => { assert.fail('must not provision'); } });
  try {
    for (const path of ['/.well-known/oauth-authorization-server', '/.well-known/oauth-protected-resource/mcp']) {
      const request = new Request(issuer + path);
      const bootstrap = await discovery(request, issuer).json();
      const live = await (await oauth.fetch(request)).json();
      for (const [key, value] of Object.entries(bootstrap)) assert.deepEqual(live[key], value);
    }
    assert.equal(discovery(new Request(issuer + '/mcp'), issuer).status, 401);
    assert.equal(discovery(new Request(issuer + '/oauth/authorize'), issuer).status, 503);
    assert.equal(discovery(new Request('https://other.example/mcp'), issuer).status, 421);
  } finally { oauth.close(); }
});
