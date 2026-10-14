import { McpServer } from '@modelcontextprotocol/server';
import * as z from 'zod/v4';
import { selectTools, inputSchemaFor } from './catalog.mjs';
import { TonkToolError } from './errors.mjs';
export { TonkToolError } from './errors.mjs';
import { instructions } from './authoring.mjs';
import { registerAuthoringResources } from './authoring-resources.mjs';

export function createTonkServer({ capabilities, call, spaceSelection }, { registerUI } = {}) {
  const server = new McpServer({ name: 'tonk', version: '0.1.0' }, { instructions });
  registerAuthoringResources(server);
  for (const tool of selectTools(capabilities)) {
    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: z.fromJSONSchema(inputSchemaFor(tool, { spaceSelection })),
        annotations: tool.annotations,
      },
      async (args, context) => {
        try {
          // Propagate both protocol cancellation and HTTP disconnects. Other
          // hosts use the SDK request signal without an HTTP dependency.
          const signal = context.http?.req?.signal
            ? AbortSignal.any([context.mcpReq.signal, context.http.req.signal])
            : context.mcpReq.signal;
          const result = await call(tool.name, args, signal);
          if (!result || typeof result !== 'object' || Array.isArray(result)) {
            throw new Error('Invalid runtime result.');
          }
          return {
            content: [{ type: 'text', text: JSON.stringify(result) }],
            structuredContent: result,
          };
        } catch (error) {
          const text =
            error instanceof TonkToolError
              ? error.message
              : tool.annotations.readOnlyHint
                ? 'The runtime read failed. This does not establish that sign-in expired. Retry the read once; if it fails again, report the failure for diagnosis.'
                : 'The write outcome is unknown. Query the space before deciding what to do. Do not repeat the write automatically.';
          return { isError: true, content: [{ type: 'text', text }] };
        }
      },
    );
  }
  registerUI?.(server);
  return server;
}
