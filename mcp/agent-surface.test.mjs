import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createAgentTools, instructions, guides } from './browser.mjs';
import { createTonkHTTP } from './http.mjs';
import { selectTools } from './catalog.mjs';

test('browser tools and MCP share definitions, results and authoring context', async () => {
  assert.equal(
    instructions,
    await readFile(new URL('../rust/tonk-cli/src/guide-agent.md', import.meta.url), 'utf8'),
  );
  assert.equal(guides.agent, instructions);
  const result = { revision_after: null, matches_after: [{ results: [{ this: 'id:test' }] }] };
  let requests = 0;
  const options = {
    subject: 'did:key:z6Test',
    request: async () => {
      requests++;
      return Response.json(result);
    },
  };
  const browser = createAgentTools(options);
  assert.deepEqual(browser.tools, selectTools(['tonk_query', 'tonk_evaluate', 'tonk_guide']));
  assert.deepEqual(await browser.call('tonk_query', { document: 'example:' }), result);
  const backend = { capabilities: browser.tools.map((t) => t.name), call: browser.call };
  const app = createTonkHTTP(async () => backend),
    client = new Client({ name: 'shared-context', version: '1' });
  try {
    await client.connect(
      new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
        fetch: (url, init) => app.fetch(new Request(url, init)),
      }),
    );
    assert.equal(client.getInstructions(), instructions);
    const resource = await client.readResource({ uri: 'tonk://guide/notation' });
    const guide = await client.callTool({ name: 'tonk_guide', arguments: { topic: 'notation' } });
    assert.equal(resource.contents[0].text, guide.structuredContent.text);
    assert.equal(guide.structuredContent.text, guides.notation);
    const query = await client.callTool({
      name: 'tonk_query',
      arguments: { document: 'example:' },
    });
    assert.deepEqual(query.structuredContent, result);
    assert.equal(requests, 2); // Reading guides does not touch the worker.
  } finally {
    await client.close();
    await app.close();
  }
});

test('browser host authority disables writes without changing the core contract', async () => {
  let requests = 0;
  const browser = createAgentTools({
    subject: 'did:key:z6Test',
    allowsWrites: false,
    request: () => {
      requests++;
    },
  });
  assert.deepEqual(
    browser.tools.map((t) => t.name),
    ['tonk_query', 'tonk_guide'],
  );
  await assert.rejects(browser.call('tonk_evaluate', { document: 'example!:' }), /read-only/);
  await assert.rejects(
    browser.call('tonk_query', { document: 'example:', url: 'https://other' }),
    /one inline/,
  );
  assert.equal(requests, 0);
});
