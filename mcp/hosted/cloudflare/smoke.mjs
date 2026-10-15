// Exercises the deployed container, not registration-only discovery at the edge.
import assert from 'node:assert/strict';
import { setTimeout } from 'node:timers/promises';
import { providerOrigin } from '../provider.mjs';

const issuer = providerOrigin(process.env.MCP_ISSUER);
const commit = process.env.BUILD_COMMIT;
assert.match(commit, /^[a-f0-9]{40}$/);
async function get(path) {
  return fetch(`${issuer}${path}`, { redirect: 'error', signal: AbortSignal.timeout(30000) });
}
async function check() {
  const build = await get('/build.json');
  assert.equal(build.status, 200);
  assert.equal((await build.json()).commit, commit);
  const authorization = await get('/.well-known/oauth-authorization-server');
  assert.equal(authorization.status, 200);
  const metadata = await authorization.json();
  assert.equal(metadata.issuer, issuer);
  assert.equal(metadata.token_endpoint, `${issuer}/oauth/token`);
  assert.ok(metadata.code_challenge_methods_supported.includes('S256'));
  const resource = await get('/.well-known/oauth-protected-resource/mcp');
  assert.equal(resource.status, 200);
  const protection = await resource.json();
  assert.equal(protection.resource, `${issuer}/mcp`);
  assert.deepEqual(protection.authorization_servers, [issuer]);
  const unauthorized = await get('/mcp');
  assert.equal(unauthorized.status, 401);
  assert.ok(
    unauthorized.headers
      .get('www-authenticate')
      .includes(`${issuer}/.well-known/oauth-protected-resource/mcp`),
  );
  const assets = await get('/space-runtime.json');
  assert.equal(assets.status, 200);
  assert.ok((await assets.json()).manifest.wasm.endsWith('.wasm'));
  const guest = await get('/worker-guest.html');
  assert.equal(guest.status, 200);
  assert.ok((await guest.text()).includes('<!doctype html>'));
}
// A new container image can need several minutes to become available.
for (let attempt = 1; ; attempt++) {
  try {
    await check();
    console.log(`Deployed MCP checks passed: ${issuer}/mcp (${commit})`);
    break;
  } catch (error) {
    if (attempt === 20) throw error;
    console.log(`Waiting for deployed MCP (${attempt}/20): ${error.message}`);
    await setTimeout(15000);
  }
}
