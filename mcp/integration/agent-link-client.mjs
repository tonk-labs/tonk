// Invoked by the native agent-link spike integration fixture. Secrets arrive
// only on stdin and are never included in assertion diagnostics.
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';
import { createAgentService } from '../hosted/agent-service.mjs';
import { createCheckpoint } from '../hosted/checkpoint.mjs';
import { mkdir, rename, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

let input = '';
for await (const chunk of process.stdin) input += chunk;
const config = JSON.parse(input);
const client = new Client({ name: 'agent-link-spike-integration', version: '1' });
await client.connect(
  new StdioClientTransport({
    command: process.execPath,
    args: [
      fileURLToPath(new URL('../scripts/agent-link.mjs', import.meta.url)),
      config.binary,
      config.directory,
      config.remote,
    ],
    stderr: 'pipe',
  }),
);
try {
  assert.deepEqual((await client.listTools()).tools.map((tool) => tool.name).sort(), [
    'tonk_evaluate',
    'tonk_query',
  ]);
  const write = await client.callTool({
    name: 'tonk_evaluate',
    arguments: {
      agent_link: config.link,
      document: 'spike-record!:\n  this: urn:spike:mcp\n  title: Written through stdio MCP\n',
    },
  });
  assert.notEqual(write.isError, true, 'MCP write failed');
  const read = await client.callTool({
    name: 'tonk_query',
    arguments: { agent_link: config.link, document: 'spike-record:\n' },
  });
  assert.notEqual(read.isError, true, 'MCP query failed');
  assert.ok(JSON.stringify(read.structuredContent).includes('Written through stdio MCP'));
  const missing = await client.callTool({
    name: 'tonk_query',
    arguments: { document: 'spike-record:\n' },
  });
  assert.equal(missing.isError, true);
} finally {
  await client.close();
}
console.log('Agent-link stdio MCP write and query passed.');

let snapshot, etag;
const checkpointFetch = async (_url, init = {}) => {
  if (init.method === 'PUT') {
    assert.equal(init.headers['if-match'], etag);
    snapshot = Buffer.from(init.body);
    etag = '"fixture"';
    return new Response(null, { headers: { etag } });
  }
  return snapshot
    ? new Response(snapshot, { headers: { etag } })
    : new Response(null, { status: 404 });
};
const hostedDirectory = config.directory + '-http';
await mkdir(hostedDirectory, { mode: 0o700 });
const checkpoint = await createCheckpoint({ directory: hostedDirectory, fetch: checkpointFetch });
const service = await createAgentService({
  issuer: 'https://mcp.example',
  provider: new URL(config.remote).origin,
  binary: config.binary,
  directory: hostedDirectory,
  checkpoint,
});
const hosted = new Client({ name: 'hosted-agent-integration', version: '1' });
try {
  await hosted.connect(
    new StreamableHTTPClientTransport(new URL('https://mcp.example/mcp/link'), {
      fetch: (url, init) => service.fetch(new Request(url, init)),
    }),
  );
  const result = await hosted.callTool({
    name: 'tonk_evaluate',
    arguments: {
      agent_link: config.link,
      document: 'spike-record!:\n  this: urn:spike:http\n  title: Written through HTTP MCP\n',
    },
  });
  assert.notEqual(result.isError, true, 'HTTP write failed');
  const read = await hosted.callTool({
    name: 'tonk_query',
    arguments: {
      agent_link: config.link,
      document: 'spike-record:\n',
    },
  });
  assert.notEqual(read.isError, true);
  assert.ok(JSON.stringify(read.structuredContent).includes('Written through HTTP MCP'));
} finally {
  await hosted.close();
  await service.close();
}

assert.ok(snapshot?.length > 0, 'native replicas were not checkpointed');
// A restarted container cannot reach the previous root. Keep its disposable
// fixture bytes for diagnosis, but make the recorded absolute paths unavailable.
await rename(hostedDirectory, hostedDirectory + '-retired');
await assert.rejects(stat(hostedDirectory), { code: 'ENOENT' });
const restoredDirectory = config.directory + '-restored';
await mkdir(restoredDirectory, { mode: 0o700 });
const restoredCheckpoint = await createCheckpoint({
  directory: restoredDirectory,
  fetch: checkpointFetch,
});
const restored = await createAgentService({
  issuer: 'https://mcp.example',
  provider: new URL(config.remote).origin,
  binary: config.binary,
  directory: restoredDirectory,
  checkpoint: restoredCheckpoint,
});
const afterRestart = new Client({ name: 'restored-agent-integration', version: '1' });
try {
  await afterRestart.connect(
    new StreamableHTTPClientTransport(new URL('https://mcp.example/mcp/link'), {
      fetch: (url, init) => restored.fetch(new Request(url, init)),
    }),
  );
  const read = await afterRestart.callTool({
    name: 'tonk_query',
    arguments: {
      agent_link: config.link,
      document: 'spike-record:\n',
    },
  });
  assert.notEqual(read.isError, true, 'restored query failed');
  assert.ok(JSON.stringify(read.structuredContent).includes('Written through HTTP MCP'));
} finally {
  await afterRestart.close();
  await restored.close();
}
