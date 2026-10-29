import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import * as z from 'zod/v4';
import { TonkToolError } from '../errors.mjs';

const template = readFileSync(new URL('./ui/worker-space.html', import.meta.url), 'utf8');

// Host configuration only. Tool arguments cannot choose where credentials go.
export function renderWorkerUI({ origin }) {
  const url = new URL(origin);
  const local = url.protocol === 'http:' && ['127.0.0.1', 'localhost'].includes(url.hostname);
  if (url.origin !== origin || (url.protocol !== 'https:' && !local)) {
    throw new Error('Expected an HTTPS UI origin or a loopback development origin.');
  }
  return template.replace('__TONK_MCP_ORIGIN__', JSON.stringify(origin));
}

// `entity@model!view`, as tonk_outline writes a node's route.
export function parseViewRoute(route) {
  const invalid = () => new TonkToolError('Provide a route from tonk_outline.');
  if (typeof route !== 'string' || route.length > 2048 || /[\x00-\x1f<>"]/.test(route))
    throw invalid();
  const bang = route.indexOf('!');
  const head = bang < 0 ? route : route.slice(0, bang);
  const facet = bang < 0 ? undefined : route.slice(bang + 1);
  const at = head.indexOf('@');
  const entity = at < 0 ? undefined : head.slice(0, at);
  const model = at < 0 ? head : head.slice(at + 1);
  if (!model || entity === '' || facet === '') throw invalid();
  return { model, ...(entity ? { entity } : {}), ...(facet ? { facet } : {}) };
}

export function createWorkerUI({ origin, resourceMetadata = {} }) {
  const html = renderWorkerUI({ origin });
  // Include deployment configuration in the cache key as well as the source.
  const digest = createHash('sha256').update(html).digest('hex').slice(0, 16);
  const uri = `ui://tonk/worker-space-${digest}.html`;

  // `tools: false` registers the page alone. A host may read it in a request
  // that does not repeat the client's Apps capability.
  return function registerWorkerUI(server, backend, { tools = true } = {}) {
    if (!backend.openSpace) return;
    server.registerResource(
      'tonk-space',
      uri,
      { mimeType: 'text/html;profile=mcp-app' },
      async () => ({
        contents: [
          {
            uri,
            mimeType: 'text/html;profile=mcp-app',
            text: html,
            _meta: {
              ui: { csp: { connectDomains: [origin], frameDomains: [origin] } },
              ...resourceMetadata,
            },
          },
        ],
      }),
    );
    if (!tools) return;
    server.registerTool(
      'tonk_open_space',
      {
        description:
          'Open the connected Tonk space in its normal frontend. Use a path within this space, such as / or /notebook/<entity>. Entity route parameters must be actual entity URIs returned by tonk_query, never titles or names. For a notebook requested by title, query notebook/named to obtain its this entity, then open /notebook/<that entity>. Opening the frontend does not verify that a route rendered successfully. The user and agent operate on the same worker.',
        inputSchema: z
          .object({
            ...(backend.spaceSelection
              ? {
                  space: z
                    .string()
                    .regex(/^did:key:[A-Za-z0-9]+$/)
                    .describe('Exact subject from tonk_list_spaces.'),
                }
              : {}),
            path: z.string().min(1).max(4096).default('/'),
          })
          .strict(),
        annotations: { readOnlyHint: true, openWorldHint: false },
        _meta: { ui: { resourceUri: uri } },
      },
      async ({ path, space }) => {
        try {
          if (!path.startsWith('/') || path.startsWith('//') || /[\\\x00-\x1f]/.test(path)) {
            throw new TonkToolError('Provide a path within the connected space.');
          }
          const session = await backend.openSpace(space);
          return {
            content: [
              {
                type: 'text',
                text: 'Requested the Tonk frontend at the supplied path. Route rendering is not verified by this tool response.',
              },
            ],
            structuredContent: { subject: space ?? backend.subject, path },
            _meta: { session },
          };
        } catch (error) {
          return {
            isError: true,
            content: [
              {
                type: 'text',
                text:
                  error instanceof TonkToolError
                    ? error.message
                    : 'The Tonk frontend could not be opened. Retry this read; reconnecting is not yet indicated.',
              },
            ],
          };
        }
      },
    );
    const spaceArgument = backend.spaceSelection
      ? {
          space: z
            .string()
            .regex(/^did:key:[A-Za-z0-9]+$/)
            .describe('Exact subject from tonk_list_spaces.'),
        }
      : {};
    server.registerTool(
      'tonk_show_view',
      {
        description:
          'Show one view of a Tonk space inline in the chat, on its own and live: for example the to-do list inside a trip planner, which the user can check off. Get the route from tonk_outline; never guess it. Changes the user makes in the view are written by you, the agent, not signed by the user. Showing a view does not verify that it rendered.',
        inputSchema: z
          .object({
            ...spaceArgument,
            route: z
              .string()
              .min(1)
              .max(2048)
              .describe('A node route from tonk_outline, as entity@model or model.'),
          })
          .strict(),
        annotations: { readOnlyHint: true, openWorldHint: false },
        _meta: { ui: { resourceUri: uri } },
      },
      async ({ route, space }) => {
        try {
          const view = parseViewRoute(route);
          const session = await backend.openSpace(space);
          return {
            content: [
              {
                type: 'text',
                text: 'Requested the view in the chat. Rendering is not verified by this tool response.',
              },
            ],
            structuredContent: { subject: space ?? backend.subject, path: '/', view },
            _meta: { session },
          };
        } catch (error) {
          return {
            isError: true,
            content: [
              {
                type: 'text',
                text:
                  error instanceof TonkToolError
                    ? error.message
                    : 'The view could not be opened. Retry this read; reconnecting is not yet indicated.',
              },
            ],
          };
        }
      },
    );
  };
}
