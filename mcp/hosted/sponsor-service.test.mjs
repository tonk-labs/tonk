import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createSponsorService } from './sponsor-service.mjs';
import { sponsorStubProvisioner, throwawayDid } from './sponsor-stub.mjs';

const issuer = 'https://mcp.tonk.example';
const provider = 'https://app.tonk.example';
const resource = `${issuer}/mcp`;
const chatgpt = 'https://chatgpt.com/connector_platform_oauth_redirect';
const hash = (value) => createHash('sha256').update(value).digest('base64url');
// A real tonk-identity delegation; see delegation-parties.test.mjs.
const sponsor = 'did:key:z6Mkon3Necd6NkkyfoGoHxid2znGc59LU3K7mubaRcFbLfLX';
const agent = 'did:key:z6Mko9hTggMwjSTEaJaPUfE6tqcy2xvU6BnNq3e3o8qVBiyH';
const delegationHex =
  'a16663746e2d7631815901368258405e060f9df267938da7fbf7ed550d3310fa84fe2da72bb680fe5915047e1d28e6f4f3b1a979f3c4ff8cf1cf33176841450821181eb4430bcb31b860836089700fa26168483401ed01ed011371737563616e2f646c6740312e302e302d72632e31a86361756478386469643a6b65793a7a364d6b6f39685467674d776a535445614a615055664536747163793278765536426e4e713365336f3871564269794863636d64612f63657870f66369737378386469643a6b65793a7a364d6b6f6e334e656364364e6b6b79666f476f48786964327a6e476335394c55334b376d756261526346624c664c5863706f6c8063737562f6646d657461a16c686f6d652e616464726573737668747470733a2f2f6875622e746573742f7563616e2f656e6f6e63655044db865ee2e4f2fa48841888a0668a44';

async function service({ mint = () => agent } = {}) {
  const durable = { state: undefined, run: (operation) => operation() };
  return createSponsorService({
    issuer,
    provider,
    durable,
    provision: sponsorStubProvisioner({ mint }),
  });
}

// Drive a browser through authorize -> approval page -> callback, the way a
// client and the Tonk app would, and return the issued token response.
async function connect(svc, { approve = delegationHex } = {}) {
  const registration = await svc.fetch(
    new Request(`${issuer}/oauth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        client_name: 'ChatGPT',
        redirect_uris: [chatgpt],
        grant_types: ['authorization_code', 'refresh_token'],
        response_types: ['code'],
        token_endpoint_auth_method: 'none',
        // Extra metadata a real client sends.
        client_uri: 'https://chatgpt.com',
        logo_uri: 'https://chatgpt.com/logo.png',
      }),
    }),
  );
  assert.equal(registration.status, 201);
  const { client_id } = await registration.json();

  const verifier = randomBytes(32).toString('base64url');
  const start = await svc.fetch(
    new Request(
      `${issuer}/oauth/authorize?` +
        new URLSearchParams({
          client_id,
          redirect_uri: chatgpt,
          response_type: 'code',
          resource,
          state: 'client-state',
          code_challenge: hash(verifier),
          code_challenge_method: 'S256',
        }),
    ),
  );
  assert.equal(start.status, 303);
  const cookie = start.headers.get('set-cookie').split(';')[0];
  const go = await svc.fetch(new Request(start.headers.get('location'), { headers: { cookie } }));
  assert.equal(go.status, 303);
  const approval = new URL(go.headers.get('location'));
  assert.equal(approval.origin + approval.pathname, `${provider}/settings/link`);
  assert.equal(approval.searchParams.get('name'), 'Unverified client: ChatGPT');
  const callback = new URL(approval.searchParams.get('callback'));
  const done = await svc.fetch(
    new Request(`${issuer}/oauth/callback`, {
      method: 'POST',
      headers: { cookie, origin: issuer, 'content-type': 'application/json' },
      body: JSON.stringify({
        request: callback.searchParams.get('request'),
        authorize: Buffer.from(JSON.stringify({ delegationHex: approve })).toString('base64'),
      }),
    }),
  );
  const result = await done.json();
  if (!result.redirect) return { approval, result };
  const redirect = new URL(result.redirect);
  assert.equal(redirect.searchParams.get('state'), 'client-state');
  assert.equal(redirect.searchParams.get('iss'), issuer);
  const token = await svc.fetch(
    new Request(`${issuer}/oauth/token`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'authorization_code',
        client_id,
        code: redirect.searchParams.get('code'),
        redirect_uri: chatgpt,
        code_verifier: verifier,
        resource,
      }),
    }),
  );
  assert.equal(token.status, 200);
  return { approval, token: await token.json() };
}

test('it challenges an unauthenticated MCP request and publishes discovery', async () => {
  const svc = await service();
  try {
    const challenge = await svc.fetch(new Request(resource, { method: 'POST' }));
    assert.equal(challenge.status, 401);
    assert.match(
      challenge.headers.get('www-authenticate'),
      /resource_metadata="https:\/\/mcp\.tonk\.example\/\.well-known\/oauth-protected-resource\/mcp"/,
    );
    const protectedResource = await svc.fetch(
      new Request(`${issuer}/.well-known/oauth-protected-resource/mcp`),
    );
    assert.equal((await protectedResource.json()).resource, resource);
    const server = await (
      await svc.fetch(new Request(`${issuer}/.well-known/oauth-authorization-server`))
    ).json();
    assert.equal(server.issuer, issuer);
    assert.deepEqual(server.code_challenge_methods_supported, ['S256']);
    assert.equal(server.registration_endpoint, `${issuer}/oauth/register`);
  } finally {
    await svc.close();
  }
});

test('a sponsor approves a stub agent and the agent sees who sponsored it', async () => {
  const svc = await service();
  try {
    const { approval, token } = await connect(svc);
    assert.equal(approval.searchParams.get('audience'), agent);
    const client = new Client({ name: 'sponsor-test', version: '1' });
    await client.connect(
      new StreamableHTTPClientTransport(new URL(resource), {
        fetch: (url, init) => svc.fetch(new Request(url, init)),
        requestInit: { headers: { authorization: `Bearer ${token.access_token}` } },
      }),
    );
    try {
      const tools = (await client.listTools()).tools.map((tool) => tool.name).sort();
      assert.deepEqual(tools, ['tonk_guide', 'tonk_list_spaces']);
      const listed = await client.callTool({ name: 'tonk_list_spaces', arguments: {} });
      assert.deepEqual(listed.structuredContent.spaces, []);
      assert.equal(listed.structuredContent.agent, agent);
      assert.equal(listed.structuredContent.sponsor, sponsor);
    } finally {
      await client.close();
    }
  } finally {
    await svc.close();
  }
});

test('it refuses an approval that names a different agent', async () => {
  // The service mints its own agent; the fixture delegation is to another key.
  const svc = await service({ mint: throwawayDid });
  try {
    const { result } = await connect(svc);
    assert.deepEqual(result, { failure: 'device-check' });
  } finally {
    await svc.close();
  }
});

test('a throwaway agent DID is a fresh Ed25519 did:key', () => {
  const a = throwawayDid(),
    b = throwawayDid();
  assert.match(a, /^did:key:z6Mk[1-9A-HJ-NP-Za-km-z]{44}$/);
  assert.notEqual(a, b);
});
