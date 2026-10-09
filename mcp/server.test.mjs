import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, writeFile, rm, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';
import { connectRuntime } from './runtime-client.mjs';

for (const supportsInspection of [false, true])
for (const allowsWrites of [false, true]) test(`external stdio client with writes=${allowsWrites}, inspection=${supportsInspection}`, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tonk-mcp-test-'));
  const token = 'a'.repeat(72);
  const calls = [];
  const names = ['tonk_query', 'tonk_preview', ...(allowsWrites ? ['tonk_apply'] : []), ...(supportsInspection ? ['tonk_inspect_view'] : [])];
  const host = createServer(async (req, res) => {
    assert.equal(req.headers.authorization, `Bearer ${token}`);
    let body = '';
    for await (const chunk of req) body += chunk;
    const value = JSON.parse(body);
    res.setHeader('content-type', 'application/json');
    if (req.url === '/tools') {
      res.end(JSON.stringify({ tools: names.map(name => ({
        name, description: 'Fixture tool', annotations: { readOnlyHint: name !== 'tonk_apply' },
        inputSchema: { type: 'object', additionalProperties: false,
          properties: { ...(name === 'tonk_inspect_view' ? {} : { [name === 'tonk_query' ? 'target' : 'document']: { type: 'string' } }),
            ...(name === 'tonk_apply' ? { expectedRevision: { anyOf: [{ type: 'object', additionalProperties: true }, { type: 'null' }] } } : {}) },
          required: name === 'tonk_inspect_view' ? [] : name === 'tonk_apply' ? ['document', 'expectedRevision'] : [name === 'tonk_query' ? 'target' : 'document'] },
      })) }));
    } else {
      calls.push(value);
      if (value.name === 'tonk_apply') { req.socket.destroy(); return; }
      if (value.name === 'tonk_inspect_view') { res.end(JSON.stringify({result: {frames: [{text: 'Fixture checklist'}], renderedRevision: null, revisionTracking: 'unavailable'}})); return; }
      res.end(JSON.stringify(value.arguments.document === 'bad'
        ? { error: 'Notation failed validation.' }
        : { result: { committed: false, revision: 'fixture', matches: [] } }));
    }
  });
  await new Promise(resolve => host.listen(0, '127.0.0.1', resolve));
  const config = join(directory, 'connection.json');
  await writeFile(config, JSON.stringify({ version: 1, url: `http://127.0.0.1:${host.address().port}`, token }), { mode: 0o600 });
  const client = new Client({ name: 'independent-test-client', version: '1.0.0' });
  try {
    await client.connect(new StdioClientTransport({
      command: process.execPath,
      args: [fileURLToPath(new URL('./server.mjs', import.meta.url)), config],
      stderr: 'pipe',
    }));
    assert.deepEqual((await client.listTools()).tools.map(tool => tool.name).sort(), names.toSorted());
    for (const [name, args] of [['tonk_query', { document: 'packing-item:\n' }], ['tonk_preview', { document: 'packing-item:\n' }]]) {
      const result = await client.callTool({ name, arguments: args });
      assert.equal(result.isError, undefined);
      assert.deepEqual(result.structuredContent, { committed: false, revision: 'fixture', matches: [] });
    }
    const failed = await client.callTool({ name: 'tonk_preview', arguments: { document: 'bad' } });
    assert.equal(failed.isError, true);
    assert.match(failed.content[0].text, /failed validation/);
    assert.equal(calls.length, 3);
    const invalid = await client.callTool({ name: 'tonk_query', arguments: { document: 'thing:\n', space: 'other' } });
    assert.equal(invalid.isError, true);
    assert.equal(calls.length, 3, 'invalid arguments must not reach host');
    if (allowsWrites) {
      for (const expectedRevision of [null, { tree: 'opaque-tree', issuer: 'did:key:fixture', signature: [1, 2, 3] }]) {
        const count = calls.length;
        const uncertain = await client.callTool({ name: 'tonk_apply', arguments: { document: 'thing!:', expectedRevision } });
        assert.equal(uncertain.isError, true);
        assert.match(uncertain.content[0].text, /Do not repeat/);
        assert.equal(calls.length, count + 1, 'lost response must not replay write');
        assert.deepEqual(calls.at(-1).arguments.expectedRevision, expectedRevision, 'SDK must preserve opaque revision fields');
      }
    }
    if (supportsInspection) {
      const inspected = await client.callTool({ name: 'tonk_inspect_view', arguments: {} });
      assert.equal(inspected.isError, undefined);
      assert.equal(inspected.structuredContent.frames[0].text, 'Fixture checklist');
      assert.equal(inspected.structuredContent.renderedRevision, null);
      assert.equal(inspected.structuredContent.revisionTracking, 'unavailable');
      const count = calls.length;
      const invalidInspection = await client.callTool({ name: 'tonk_inspect_view', arguments: { space: 'other' } });
      assert.equal(invalidInspection.isError, true);
      assert.equal(calls.length, count);
    }
    await chmod(config, 0o644);
    await assert.rejects(connectRuntime(config), /private file/);
  } finally {
    await client.close();
    await new Promise(resolve => host.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
});
