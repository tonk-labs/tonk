import { test } from 'node:test';
import assert from 'node:assert/strict';
import { openAIResourceMetadata } from './metadata.mjs';
test('OpenAI compatibility adapter allows the MCP origin for requests and frames no site', () => {
  assert.deepEqual(openAIResourceMetadata('https://mcp.example'), {
    'openai/widgetCSP': {
      connect_domains: ['https://mcp.example'],
      frame_domains: [],
      resource_domains: [],
    },
  });
});
