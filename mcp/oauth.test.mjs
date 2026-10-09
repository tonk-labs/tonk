import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createTonkOAuth } from './oauth.mjs';
import { createAuthenticatedTonkHTTP } from './authenticated-http.mjs';

const issuer = 'https://mcp.tonk.example';
const redirect = 'https://chatgpt.com/connector/oauth/test';
const resource = `${issuer}/mcp`;
const hash = value => createHash('sha256').update(value).digest('base64url');
function fixture(options = {}) {
  let time = 1_000_000, created = 0, activated = 0;
  const oauth = createTonkOAuth({ issuer, clientId: 'chatgpt-test', redirectUris: [redirect],
    now: () => time,
    provisionAccount: async () => {
      const tenantId = `tenant-${++created}`, deviceDid = `did:key:device-${created}`;
      return { tenantId, deviceDid, async authorize(payload) {
        activated++;
        if (payload.grant !== deviceDid) throw new Error('Invalid test grant.');
        return { deviceDid, rootDid: `did:key:account-${tenantId}` };
      } };
    }, ...options,
  });
  async function start(overrides = {}) {
    const verifier = randomBytes(32).toString('base64url');
    const params = new URLSearchParams({ client_id: 'chatgpt-test', redirect_uri: redirect,
      response_type: 'code', scope: 'tonk', resource, state: 'original-client-state',
      code_challenge: hash(verifier), code_challenge_method: 'S256', ...overrides });
    const response = await oauth.fetch(new Request(`${issuer}/oauth/authorize?${params}`));
    return { response, verifier, cookie: response.headers.get('set-cookie')?.split(';')[0],
      location: response.headers.get('location') };
  }
  async function begin() {
    const flow = await start();
    assert.equal(flow.response.status, 303);
    assert.match(flow.response.headers.get('set-cookie'), /Secure; HttpOnly; SameSite=Lax/);
    const response = await oauth.fetch(new Request(flow.location, { headers: { cookie: flow.cookie } }));
    assert.equal(response.status, 303);
    const approval = new URL(response.headers.get('location'));
    return { ...flow, deviceDid: approval.searchParams.get('audience'),
      callback: approval.searchParams.get('callback'), approval };
  }
  async function callback(flow, changes = {}, headers = {}) {
    const authorize = Buffer.from(JSON.stringify({ grant: flow.deviceDid })).toString('base64');
    return oauth.fetch(new Request(`${issuer}/oauth/callback`, { method: 'POST',
      headers: { cookie: flow.cookie, origin: issuer, 'content-type': 'application/json', ...headers },
      body: JSON.stringify({ request: new URL(flow.callback).searchParams.get('request'), authorize, deny: null, ...changes }),
    }));
  }
  async function code(flow) {
    const response = await callback(flow);
    assert.equal(response.status, 200);
    const target = new URL((await response.json()).redirect);
    assert.equal(target.origin + target.pathname, redirect);
    assert.equal(target.searchParams.get('state'), 'original-client-state');
    assert.equal(target.searchParams.get('iss'), issuer);
    return target.searchParams.get('code');
  }
  async function exchange(flow, code, changes = {}) {
    return oauth.fetch(new Request(`${issuer}/oauth/token`, { method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ grant_type: 'authorization_code', client_id: 'chatgpt-test',
        redirect_uri: redirect, resource, code, code_verifier: flow.verifier, ...changes }),
    }));
  }
  return { oauth, start, begin, callback, code, exchange,
    advance: ms => { time += ms; }, counts: () => ({ created, activated }) };
}

test('OAuth discovery, Tonk approval, PKCE exchange and expiration', async () => {
  const f = fixture();
  const metadata = await (await f.oauth.fetch(new Request(`${issuer}/.well-known/oauth-authorization-server`))).json();
  assert.deepEqual(metadata.code_challenge_methods_supported, ['S256']);
  assert.deepEqual(metadata.token_endpoint_auth_methods_supported, ['none']);
  const protectedResource = await (await f.oauth.fetch(new Request(`${issuer}/.well-known/oauth-protected-resource/mcp`))).json();
  assert.equal(protectedResource.resource, resource);
  const flow = await f.begin();
  assert.equal(flow.approval.origin, 'https://tonk.network');
  const page = await f.oauth.fetch(new Request(flow.callback, { headers: { cookie: flow.cookie } }));
  assert.match(page.headers.get('content-security-policy'), /frame-ancestors 'none'/);
  const html = await page.text();
  assert.ok(html.indexOf('history.replaceState') < html.indexOf("fetch('/oauth/callback'"));
  assert.equal(html.includes('original-client-state'), false);
  const code = await f.code(flow);
  const result = await f.exchange(flow, code);
  assert.equal(result.status, 200);
  const token = await result.json();
  assert.equal(token.token_type, 'Bearer');
  assert.match(token.refresh_token, /^[A-Za-z0-9_-]{43}$/);
  const request = new Request(resource, { headers: { authorization: `Bearer ${token.access_token}` } });
  assert.equal(f.oauth.authenticate(request).tenantId, 'tenant-1');
  assert.equal(f.oauth.authenticate(new Request(`${resource}?tenant=other`, { headers: request.headers })), undefined);
  assert.equal((await f.exchange(flow, code)).status, 400);
  assert.equal(f.oauth.authenticate(request), undefined, 'reusing a code revokes its minted token');
  const next = await f.begin(), nextCode = await f.code(next);
  const nextToken = await (await f.exchange(next, nextCode)).json();
  const nextRequest = new Request(resource, { headers: { authorization: `Bearer ${nextToken.access_token}` } });
  assert.ok(f.oauth.authenticate(nextRequest));
  f.advance(600_001);
  assert.equal(f.oauth.authenticate(nextRequest), undefined);
});

test('invalid clients never redirect; registered clients receive issuer-bound errors without provisioning', async () => {
  const f = fixture();
  for (const overrides of [{ client_id: 'other' }, { redirect_uri: `${redirect}/other` },
    { redirect_uri: 'https://attacker.example/' }, { resource: 'https://other.example/mcp' },
    { code_challenge_method: 'plain' }, { code_challenge: 'short' }, { state: '' },
    { response_type: 'token' }, { scope: 'tonk admin' }]) {
    const { response } = await f.start(overrides);
    if (overrides.client_id || overrides.redirect_uri || overrides.state === '') {
      assert.equal(response.status, 400);
      assert.equal(response.headers.has('location'), false);
    } else {
      assert.equal(response.status, 303);
      const target = new URL(response.headers.get('location'));
      assert.equal(target.origin + target.pathname, redirect);
      assert.equal(target.searchParams.get('error'), 'invalid_request');
      assert.equal(target.searchParams.get('state'), 'original-client-state');
      assert.equal(target.searchParams.get('iss'), issuer);
    }
  }
  assert.deepEqual(f.counts(), { created: 0, activated: 0 });
});

test('browser cookie, origin and one-use callback prevent cross-session delivery', async () => {
  const f = fixture(), a = await f.begin(), b = await f.begin();
  for (const headers of [{ cookie: b.cookie }, { cookie: '' }, { origin: 'https://evil.example' },
    { cookie: `${a.cookie}; ${a.cookie}` }]) {
    assert.equal((await f.callback(a, {}, headers)).status, 403);
  }
  assert.equal(f.counts().activated, 0);
  await f.code(a);
  assert.equal((await f.callback(a)).status, 403);
  assert.equal(f.counts().activated, 1);
  const denied = await f.callback(b, { authorize: null, deny: 'User declined.' });
  const target = new URL((await denied.json()).redirect);
  assert.equal(target.searchParams.get('error'), 'access_denied');
  assert.equal(target.searchParams.get('iss'), issuer);
  assert.equal(f.counts().activated, 1);
});

test('code binds PKCE, client, redirect and resource; expiry and shutdown invalidate credentials', async () => {
  const f = fixture(), flow = await f.begin(), code = await f.code(flow);
  for (const changes of [{ code_verifier: 'x'.repeat(43) }, { client_id: 'other' },
    { redirect_uri: `${redirect}/other` }, { resource: `${resource}/other` }, { client_secret: 'secret' }]) {
    assert.equal((await f.exchange(flow, code, changes)).status, 400);
  }
  const [a, b] = await Promise.all([f.exchange(flow, code), f.exchange(flow, code)]);
  assert.deepEqual([a.status, b.status].sort(), [200, 400]);
  const token = await (a.status === 200 ? a : b).json();
  f.oauth.close();
  assert.equal(f.oauth.authenticate(new Request(resource, { headers: { authorization: `Bearer ${token.access_token}` } })), undefined);
  const expired = fixture(), pending = await expired.begin();
  expired.advance(300_001);
  assert.equal((await expired.callback(pending)).status, 403);
  const c = fixture(), cFlow = await c.begin(), cCode = await c.code(cFlow);
  c.advance(60_001);
  assert.equal((await c.exchange(cFlow, cCode)).status, 400);
});

test('bounded ceremony capacity and malformed delivery fail closed', async () => {
  const f = fixture({ capacity: 1 }), flow = await f.begin();
  assert.equal((await f.start()).response.status, 503);
  assert.equal((await f.callback(flow, { authorize: 'not base64' })).status, 400);
  assert.equal(f.counts().activated, 0);
  assert.equal((await f.callback(flow)).status, 403);
  f.advance(300_001);
  assert.equal((await f.start()).response.status, 303);
});

test('authenticated HTTP checks each request and cannot cross tenant authority', async () => {
  const f = fixture();
  const seen = [];
  const app = createAuthenticatedTonkHTTP({ oauth: f.oauth, resolveBackend: async principal => {
    seen.push(principal.tenantId);
    return { capabilities: ['tonk_query'], call: async () => ({ tenant: principal.tenantId }) };
  } });
  const unauthenticated = await app.fetch(new Request(resource, { method: 'POST' }));
  assert.equal(unauthenticated.status, 401);
  assert.match(unauthenticated.headers.get('www-authenticate'), /oauth-protected-resource\/mcp/);
  assert.equal(seen.length, 0);
  for (let i = 1; i <= 2; i++) {
    const flow = await f.begin(), code = await f.code(flow);
    const { access_token: token } = await (await f.exchange(flow, code)).json();
    const client = new Client({ name: 'oauth-test', version: '1.0.0' });
    try {
      await client.connect(new StreamableHTTPClientTransport(new URL(resource), {
        fetch: (url, init) => {
          const headers = new Headers(init?.headers);
          headers.set('authorization', `Bearer ${token}`);
          return app.fetch(new Request(url, { ...init, headers }));
        },
      }));
      const response = await client.callTool({ name: 'tonk_query', arguments: { document: 'book:\n' } });
      assert.equal(response.structuredContent.tenant, `tenant-${i}`);
    } finally { await client.close(); }
  }
  assert.ok(seen.includes('tenant-1') && seen.includes('tenant-2'));
  await app.close();
});

test('concurrent callbacks cannot replay activation or free its capacity slot', async () => {
  let release, entered, calls = 0;
  const waiting = new Promise(resolve => { entered = resolve; });
  const f = fixture({ capacity: 1, provisionAccount: async () => ({
    tenantId: 'tenant-test', deviceDid: 'did:key:device-test',
    authorize: async () => {
      calls++; entered();
      await new Promise(resolve => { release = resolve; });
      return { deviceDid: 'did:key:device-test', rootDid: 'did:key:account-test' };
    },
  }) });
  const flow = await f.begin();
  const first = f.callback(flow);
  await waiting;
  assert.equal((await f.callback(flow)).status, 403);
  assert.equal((await f.start()).response.status, 503);
  release();
  assert.equal((await first).status, 200);
  assert.equal(calls, 1);
});

test('activation failure reports only an allowlisted stage, never upstream secrets', async () => {
  for (const stage of ['root-import', 'account-hydration', 'checkpoint', 'secret-grant']) {
    const f = fixture({ provisionAccount: async () => ({ tenantId: 'tenant-test', deviceDid: 'did:key:test',
      authorize: async () => { throw Object.assign(Error('secret-grant'), { authorizationStage: stage }); },
    }) });
    const flow = await f.begin();
    const response = await f.callback(flow);
    assert.deepEqual(await response.json(), { failure: stage === 'secret-grant' ? 'account-activation' : stage });
    assert.equal((await f.callback(flow)).status, 403);
    f.oauth.close();
  }
});

test('parallel browser attempts keep independent cookies and both can finish', async () => {
  const f = fixture(), a = await f.begin(), b = await f.begin();
  assert.notEqual(a.cookie.split('=')[0], b.cookie.split('=')[0]);
  const cookie = `${a.cookie}; ${b.cookie}`;
  for (const flow of [a, b]) {
    const response = await f.callback(flow, {}, { cookie });
    const target = new URL((await response.json()).redirect);
    assert.ok(target.searchParams.get('code'));
    assert.match(response.headers.get('set-cookie'), new RegExp('^' + flow.cookie.split('=')[0] + '='));
  }
  f.oauth.close();
});

test('callback navigation explains missing and expired binding and clears fragments', async () => {
  const f = fixture(), flow = await f.begin();
  let response = await f.oauth.fetch(new Request(flow.callback));
  assert.equal(response.status, 403);
  assert.match(await response.text(), /cookie is missing/);
  f.advance(300_001);
  response = await f.oauth.fetch(new Request(flow.callback, { headers: { cookie: flow.cookie } }));
  const html = await response.text();
  assert.match(html, /expired or the service restarted/);
  assert.match(html, /history.replaceState/);
  assert.equal(html.includes(new URL(flow.callback).searchParams.get('request')), false);
  f.oauth.close();
});

test('OAuth refresh is client/resource/scope bound, rotates, and revokes on replay',async()=>{
 const f=fixture(),flow=await f.begin(),code=await f.code(flow);
 const first=await(await f.exchange(flow,code)).json();
 const renew=(token,changes={})=>f.oauth.fetch(new Request(issuer+'/oauth/token',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'refresh_token',client_id:'chatgpt-test',resource,refresh_token:token,...changes})}));
 for(const changes of [{client_id:'other'},{resource:resource+'/other'},{scope:'admin'}])assert.equal((await renew(first.refresh_token,changes)).status,400);
 f.advance(600001);
 assert.equal(f.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+first.access_token}})),undefined);
 const response=await renew(first.refresh_token);assert.equal(response.status,200);const second=await response.json();
 assert.equal(f.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+second.access_token}})).tenantId,'tenant-1');
 assert.equal((await renew(first.refresh_token)).status,400);
 assert.equal(f.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+second.access_token}})),undefined);
 assert.equal((await renew(second.refresh_token)).status,400);f.oauth.close();
});

test('refresh accepts omitted resource while retaining its originally bound audience',async()=>{
 const f=fixture(),flow=await f.begin(),code=await f.code(flow);
 const first=await(await f.exchange(flow,code)).json();
 const response=await f.oauth.fetch(new Request(issuer+'/oauth/token',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'refresh_token',client_id:'chatgpt-test',refresh_token:first.refresh_token})}));
 assert.equal(response.status,200);const next=await response.json();
 assert.equal(f.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+next.access_token}})).resource,resource);
 assert.equal(f.oauth.authenticate(new Request(resource+'/other',{headers:{authorization:'Bearer '+next.access_token}})),undefined);
 f.oauth.close();
});

test('OAuth snapshot restores access and rotation without repeating approval',async()=>{
 const first=fixture(),flow=await first.begin(),code=await first.code(flow);
 const tokens=await (await first.exchange(flow,code)).json();
 const saved=JSON.parse(JSON.stringify(first.oauth.snapshot()));
 assert.ok(!JSON.stringify(saved).includes(tokens.access_token));
 const restored=fixture({saved});
 const authenticated=restored.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+tokens.access_token}}));
 assert.equal(authenticated.tenantId,'tenant-1');
 const response=await restored.oauth.fetch(new Request(issuer+'/oauth/token',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'refresh_token',client_id:'chatgpt-test',refresh_token:tokens.refresh_token})}));
 assert.equal(response.status,200);const next=await response.json();
 assert.deepEqual(restored.counts(),{created:0,activated:0});
 // Replaying an issued code still revokes the restored successor family.
 assert.equal((await restored.exchange(flow,code)).status,400);
 assert.equal(restored.oauth.authenticate(new Request(resource,{headers:{authorization:'Bearer '+next.access_token}})),undefined);
});

test('recovery retains pending approvals, unexchanged codes and live credential families only',async()=>{
  const f=fixture();
  const first=await f.begin();
  assert.deepEqual([...f.oauth.retainedTenants()],['tenant-1']);
  const code=await f.code(first);
  assert.deepEqual([...f.oauth.retainedTenants()],['tenant-1']);
  assert.equal((await f.exchange(first,code)).status,200);
  await f.begin();
  assert.deepEqual(new Set(f.oauth.retainedTenants()),new Set(['tenant-1','tenant-2']));
  f.advance(300001);
  assert.deepEqual([...f.oauth.retainedTenants()],['tenant-1']);
  f.advance(8*60*60_000);
  assert.deepEqual([...f.oauth.retainedTenants()],[]);
});
