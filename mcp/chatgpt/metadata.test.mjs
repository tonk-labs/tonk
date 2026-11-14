import { test } from 'node:test';
import assert from 'node:assert/strict';
import { openAIResourceMetadata } from './metadata.mjs';
test('OpenAI compatibility adapter preserves the existing private resource CSP', () => {
  assert.deepEqual(openAIResourceMetadata('https://mcp.example'), {
    'openai/widgetCSP': {
      connect_domains: ['https://mcp.example'],
      frame_domains: ['https://mcp.example'],
      resource_domains: [],
    },
  });
});
