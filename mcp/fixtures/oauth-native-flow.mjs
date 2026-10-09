// Driven by the ignored Rust integration test, which signs a real disposable
// grant for the audience emitted below. No live account or HTTP listener.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import { createTonkOAuth } from '../oauth.mjs';
import { nativeAccountProvisioner } from '../oauth-native.mjs';
import { startNativeAccount } from '../native.mjs';
import { join } from 'node:path';

const [binary, dataRoot] = process.argv.slice(2);
const issuer = 'https://oauth-native.example';
const redirect = 'https://chatgpt.com/connector/oauth/test';
const input = createInterface({ input: process.stdin });
const delivery = once(input, 'line');
const oauth = createTonkOAuth({ issuer, clientId: 'test', redirectUris: [redirect],
  provisionAccount: nativeAccountProvisioner({ binary, dataRoot }) });
try {
  const verifier = randomBytes(32).toString('base64url');
  const params = new URLSearchParams({ client_id: 'test', redirect_uri: redirect,
    response_type: 'code', resource: oauth.resource, scope: 'tonk', state: 'fixture-state',
    code_challenge_method: 'S256', code_challenge: createHash('sha256').update(verifier).digest('base64url') });
  const begin = await oauth.fetch(new Request(`${issuer}/oauth/authorize?${params}`));
  assert.equal(begin.status, 303);
  const cookie = begin.headers.get('set-cookie').split(';')[0];
  const proceed = await oauth.fetch(new Request(begin.headers.get('location'), { headers: { cookie } }));
  assert.equal(proceed.status, 303);
  const approval = new URL(proceed.headers.get('location'));
  process.stdout.write(JSON.stringify({ deviceDid: approval.searchParams.get('audience') }) + '\n');
  const [line] = await delivery;
  const authorization = JSON.parse(line);
  const request = new URL(approval.searchParams.get('callback')).searchParams.get('request');
  const callback = await oauth.fetch(new Request(`${issuer}/oauth/callback`, { method: 'POST',
    headers: { cookie, origin: issuer, 'content-type': 'application/json' },
    body: JSON.stringify({ request, authorize: Buffer.from(JSON.stringify(authorization)).toString('base64'), deny: null }) }));
  assert.equal(callback.status, 200);
  const target = new URL((await callback.json()).redirect);
  assert.equal(target.searchParams.get('error'), null);
  const exchanged = await oauth.fetch(new Request(`${issuer}/oauth/token`, { method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ grant_type: 'authorization_code', client_id: 'test', redirect_uri: redirect,
      resource: oauth.resource, code: target.searchParams.get('code'), code_verifier: verifier }) }));
  assert.equal(exchanged.status, 200);
  const token = await exchanged.json();
  const principal = oauth.authenticate(new Request(oauth.resource, { headers: { authorization: `Bearer ${token.access_token}` } }));
  assert.ok(principal);
  const reopened = await startNativeAccount({ binary, dataDirectory: join(dataRoot, principal.tenantId) });
  try {
    assert.deepEqual(await reopened.status(), { deviceDid: principal.deviceDid, rootDid: principal.rootDid });
  } finally { await reopened.close(); }
  // Only public identity is emitted. Neither the OAuth token nor grant is logged.
  process.stdout.write(JSON.stringify({ deviceDid: principal.deviceDid, rootDid: principal.rootDid }) + '\n');
} finally { input.close(); oauth.close(); }
