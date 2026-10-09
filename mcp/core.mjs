import { McpServer } from '@modelcontextprotocol/server';
import * as z from 'zod/v4';
import { selectTools } from './catalog.mjs';

export function createTonkServer({ capabilities, call, spaceSelection }, { registerUI } = {}) {
  const server = new McpServer({ name: 'tonk', version: '0.1.0' });
  for (const tool of selectTools(capabilities)) {
    server.registerTool(tool.name, {
      description: tool.description,
      inputSchema: z.fromJSONSchema(spaceSelection&&!['tonk_list_spaces','tonk_guide'].includes(tool.name)?{...tool.inputSchema,properties:{...tool.inputSchema.properties,space:{type:'string',pattern:'^did:key:[A-Za-z0-9]+$',description:'Exact subject returned by tonk_list_spaces.'}},required:[...(tool.inputSchema.required??[]),'space']}:tool.inputSchema),
      annotations: tool.annotations,
    }, async (args, context) => {
      try {
        const result = await call(tool.name, args, context.signal);
        if (!result || typeof result !== 'object' || Array.isArray(result)) {
          throw new Error('Invalid runtime result.');
        }
        return {
          content: [{ type: 'text', text: JSON.stringify(result) }],
          structuredContent: result,
        };
      } catch (error) {
        const text = error instanceof TonkToolError ? error.message : tool.annotations.readOnlyHint
          ? 'The runtime read failed. This does not establish that sign-in expired. Retry the read once; if it fails again, report the failure for diagnosis.'
          : 'The write outcome is unknown. Query the space before deciding what to do. Do not repeat the write automatically.';
        return { isError: true, content: [{ type: 'text', text }] };
      }
    });
  }
  registerUI?.(server);
  return server;
}

// Explicit host errors are safe to show; transport errors may contain credentials.
export class TonkToolError extends Error {}
