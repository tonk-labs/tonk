import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
import { createTonkHTTP } from './http.mjs';
import { viewURI } from './ui.mjs';

test('HTTP client shares core tools and reads a standard MCP Apps resource', async () => {
  const calls = [];
  const backend = {
    capabilities: ['tonk_query', 'tonk_preview'],
    async call(name, args) {
      calls.push({ name, args });
      return { committed: false, revision: null, matches: [{ label: 'book', results: [] }] };
    },
  };
  const handler = createTonkHTTP(async request => {
    assert.equal(new URL(request.url).pathname, '/mcp');
    return backend;
  });
  const client = new Client({ name: 'http-test', version: '1.0.0' });
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
      fetch: (url, init) => handler.fetch(new Request(url, init)),
    }));
    const tools = (await client.listTools()).tools;
    assert.equal(tools.find(tool => tool.name === 'tonk_show_query')._meta.ui.resourceUri, viewURI);
    assert.equal(tools.some(tool => tool.name === 'tonk_apply'), false);
    const result = await client.callTool({ name: 'tonk_show_query', arguments: { document: 'book:\n' } });
    assert.deepEqual(calls, [{ name: 'tonk_query', args: { document: 'book:\n' } }]);
    assert.equal(result.structuredContent.document, 'book:\n');
    const resource = await client.readResource({ uri: viewURI });
    assert.equal(resource.contents[0].mimeType, 'text/html;profile=mcp-app');
    assert.match(resource.contents[0].text, /ui\/initialize/);
    const invalid = await client.callTool({ name: 'tonk_show_query', arguments: { document: 'book:\n', space: 'other' } });
    assert.equal(invalid.isError, true);
    assert.equal(calls.length, 1);
  } finally { await client.close(); await handler.close(); }
});

test('notebook tool exposes self-contained read-only UI and host-selected links', async () => {
  const calls = [];
  const backend = {
    capabilities: ['tonk_query', 'tonk_space_info'], webOrigin: 'https://tonk.foundation',
    async call(name, args) {
      calls.push({ name, args });
      if (name === 'tonk_space_info') return { subject: 'did:key:z6Test' };
      return { revision: 'r1', matches: [
        { label: 'notebook/named', results: [{ this: 'urn:note:one', fields: { title: 'Empty notebook' } }] },
        { label: 'notebook', results: [] }, { label: 'notebook/block', results: [] },
      ] };
    },
  };
  const handler = createTonkHTTP(async () => backend);
  const client = new Client({ name: 'notebook-test', version: '1.0.0' });
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'), {
      fetch: (url, init) => handler.fetch(new Request(url, init)),
    }));
    const tool = (await client.listTools()).tools.find(tool => tool.name === 'tonk_show_notebook');
    assert.equal(tool.annotations.readOnlyHint, true);
    const resource = (await client.readResource({ uri: tool._meta.ui.resourceUri })).contents[0];
    assert.equal(resource.mimeType, 'text/html;profile=mcp-app');
    assert.match(resource.text, /TonkProse/);
    assert.doesNotMatch(resource.text, /TONK_RENDERER/);
    assert.deepEqual(resource._meta.ui.csp, { connectDomains: [], resourceDomains: [] });
    const result = await client.callTool({ name: tool.name, arguments: { entity: 'urn:note:one' } });
    assert.equal(result.structuredContent.markdown, '');
    assert.equal(result.structuredContent.url, 'https://tonk.foundation/space/did%3Akey%3Az6Test/notebook/urn%3Anote%3Aone');
    const count = calls.length;
    const invalid = await client.callTool({ name: tool.name, arguments: { entity: 'urn:note:one', space: 'other' } });
    assert.equal(invalid.isError, true);
    assert.equal(calls.length, count);
    const table = await client.callTool({ name: 'tonk_show_query', arguments: { document: 'notebook/named:\n' } });
    assert.equal(table.structuredContent.matches[0].results[0].url, result.structuredContent.url);
  } finally { await client.close(); await handler.close(); }
});
