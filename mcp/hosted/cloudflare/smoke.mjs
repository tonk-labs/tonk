// Exercises the deployed container, not registration-only discovery at the edge.
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
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
  const retired = await get('/.well-known/oauth-authorization-server');
  assert.equal(retired.status, 404);
  const client = new Client({ name: 'deployment-smoke', version: '1' });
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL(`${issuer}/mcp`)));
    const tools = (await client.listTools()).tools;
    assert.deepEqual(tools.map((tool) => tool.name).sort(), ['tonk_evaluate', 'tonk_query']);
    for (const tool of tools) assert.ok(tool.inputSchema.required.includes('agent_link'));
    const rejected = await client.callTool({
      name: 'tonk_query',
      arguments: { document: 'concept:\n' },
    });
    assert.equal(rejected.isError, true);
  } finally {
    await client.close();
  }
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
