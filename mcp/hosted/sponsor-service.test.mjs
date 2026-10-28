import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createSponsorService } from './sponsor-service.mjs';

const issuer = 'https://mcp.tonk.example';
const provider = 'https://app.tonk.example';
const resource = `${issuer}/mcp`;
const chatgpt = 'https://chatgpt.com/connector_platform_oauth_redirect';
const hash = (value) => createHash('sha256').update(value).digest('base64url');
const sponsor = 'did:key:z6Mkon3Necd6NkkyfoGoHxid2znGc59LU3K7mubaRcFbLfLX';
const otherSponsor = 'did:key:z6Mko9hTggMwjSTEaJaPUfE6tqcy2xvU6BnNq3e3o8qVBiyH';
const recipient = 'did:key:z6LSbysY2xFMRpGMhb7tFTLMpeuPRaqaWM1yECx2AtzE3KCc';

// The native agent store, in memory. `sponsor` accepts only the sponsorship
// the fake device below produces for that agent, the way the native worker
// verifies it against the agent's key.
function fakeAgents() {
  const agents = new Map();
  let next = 0;
  return {
    agents,
    async create() {
      next++;
      const agent = String(next).padStart(32, '0');
      const did = `did:key:z6MkAgent${String(next).padStart(40, 'A')}`;
      agents.set(agent, { agent, did });
      return { agent, did };
    },
    async identity(agent) {
      const found = agents.get(agent);
      if (!found) throw Error('Unknown agent.');
      return { ...found };
    },
    async sponsor({ agent, sponsorshipHex, recipient: key }) {
      const found = agents.get(agent);
      const [audience, account] = Buffer.from(sponsorshipHex, 'hex').toString().split('|');
      if (!found || audience !== found.did || !key) throw Error('not valid');
      found.sponsor = account;
      found.recipient = key;
      return { agent, did: found.did, sponsor: account };
    },
  };
}

// What the sponsor's device returns: the account, its recipient key, and a
// stand-in chain naming the agent and the account.
const delivery = (agentDid, account = sponsor) =>
  Buffer.from(
    JSON.stringify({
      account,
      recipient,
      sponsorshipHex: Buffer.from(`${agentDid}|${account}`).toString('hex'),
    }),
  ).toString('base64');

async function service(agents = fakeAgents()) {
  const durable = { state: undefined, run: (operation) => operation() };
  const svc = await createSponsorService({ issuer, provider, durable, agents });
  return Object.assign(svc, { agents });
}

// Drive a browser through authorize -> approval page -> callback, the way a
// client and the Tonk app would, and return the issued token response.
async function connect(svc, { account = sponsor, hint, forge } = {}) {
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
  const cookies = hint ? `${cookie}; ${hint}` : cookie;
  const go = await svc.fetch(
    new Request(start.headers.get('location'), { headers: { cookie: cookies } }),
  );
  assert.equal(go.status, 303);
  const approval = new URL(go.headers.get('location'));
  assert.equal(approval.origin + approval.pathname, `${provider}/settings/agent`);
  const agentDid = approval.searchParams.get('agent');
  assert.equal(approval.searchParams.get('name'), 'Unverified client: ChatGPT');
  const callback = new URL(approval.searchParams.get('callback'));
  const done = await svc.fetch(
    new Request(`${issuer}/oauth/callback`, {
      method: 'POST',
      headers: { cookie, origin: issuer, 'content-type': 'application/json' },
      body: JSON.stringify({
        request: callback.searchParams.get('request'),
        authorize: delivery(forge ?? agentDid, account),
      }),
    }),
  );
  const result = await done.json();
  const remembered = done.headers
    .getSetCookie()
    .map((value) => value.split(';')[0])
    .find((value) => value.startsWith('__Host-tonk-hint-'));
  if (!result.redirect) return { approval, result, remembered };
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
  return { approval, token: await token.json(), remembered };
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

async function listSpaces(svc, token) {
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
    return (await client.callTool({ name: 'tonk_list_spaces', arguments: {} })).structuredContent;
  } finally {
    await client.close();
  }
}

test('approving sponsors a new hosted agent with its own identity', async () => {
  const svc = await service();
  try {
    const { approval, token, remembered } = await connect(svc);
    const agent = [...svc.agents.agents.values()][0];
    assert.equal(approval.searchParams.get('agent'), agent.did);
    assert.equal(agent.sponsor, sponsor);
    assert.equal(agent.recipient, recipient);
    assert.ok(remembered?.endsWith(`=${agent.agent}`), 'the browser remembers the agent');
    const listed = await listSpaces(svc, token);
    assert.deepEqual(listed.spaces, []);
    assert.equal(listed.agent, agent.did);
    assert.equal(listed.sponsor, sponsor);
  } finally {
    await svc.close();
  }
});

test('a reconnect from the same browser keeps the same agent', async () => {
  const svc = await service();
  try {
    const first = await connect(svc);
    const second = await connect(svc, { hint: first.remembered });
    assert.equal(svc.agents.agents.size, 1);
    assert.equal(
      second.approval.searchParams.get('agent'),
      first.approval.searchParams.get('agent'),
    );
    assert.equal((await listSpaces(svc, second.token)).sponsor, sponsor);
  } finally {
    await svc.close();
  }
});

test('another account cannot take over a remembered agent', async () => {
  const svc = await service();
  try {
    const first = await connect(svc);
    const { result, remembered } = await connect(svc, {
      hint: first.remembered,
      account: otherSponsor,
    });
    assert.deepEqual(result, { failure: 'device-check' });
    assert.ok(remembered?.endsWith('='), 'the remembered agent is forgotten');
    // The next attempt from that browser starts a new agent.
    const fresh = await connect(svc, { account: otherSponsor });
    assert.equal(svc.agents.agents.size, 2);
    assert.equal((await listSpaces(svc, fresh.token)).sponsor, otherSponsor);
  } finally {
    await svc.close();
  }
});

test('it refuses a sponsorship of a different agent', async () => {
  const svc = await service();
  try {
    const { result } = await connect(svc, { forge: 'did:key:z6MkSomeoneElse' });
    assert.deepEqual(result, { failure: 'device-check' });
  } finally {
    await svc.close();
  }
});
