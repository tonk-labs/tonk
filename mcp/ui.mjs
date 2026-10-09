import { registerSpaceUI } from './space-ui.mjs';
import { readFileSync } from 'node:fs';
import * as z from 'zod/v4';
import { TonkToolError } from './core.mjs';
import { readNotebook, notebookURL } from './notebook.mjs';

export const viewURI = 'ui://tonk/query-v1.html';
export const notebookURI = 'ui://tonk/notebook-v1.html';
const mimeType = 'text/html;profile=mcp-app';
const html = readFileSync(new URL('./ui/query.html', import.meta.url), 'utf8');
const renderer = readFileSync(new URL('./ui/notebook-renderer.js', import.meta.url), 'utf8');
export const notebookHTML = readFileSync(new URL('./ui/notebook.html', import.meta.url), 'utf8')
  .replace('/* TONK_RENDERER */', () => renderer.replace(/<\/script/gi, '<\\/script'));

// Presentation extends the same query contract; no reading-list-specific store.
export function registerQueryUI(server, backend) {
  registerSpaceUI(server, backend);
  if (!backend.capabilities.includes('tonk_query')) return;
  if (backend.capabilities.includes('tonk_space_info')) {
    server.registerResource('tonk-notebook', notebookURI, { mimeType }, async () => ({
      contents: [{ uri: notebookURI, mimeType, text: notebookHTML,
        _meta: { ui: { prefersBorder: true, csp: { connectDomains: [], resourceDomains: [] } },
          'openai/widgetCSP': { connect_domains: [], resource_domains: [], redirect_domains: ['https://tonk.foundation'] },
          'openai/widgetDescription': 'Read-only Tonk notebook, using the shared Tonk prose renderer. Shows ordered text blocks and an Open in Tonk link. Does not execute query cells.' },
      }],
    }));
    server.registerTool('tonk_show_notebook', {
      description: 'Show an existing notebook in a read-only Tonk text view and return its direct Open in Tonk link. Use the exact entity from notebook/named or a successful creation. Reads the actual title, block sequence and text; reports unplaced or missing blocks. Refresh reads again. Use after creating a notebook so the user can find it. Does not execute live query cells, edit records, change the space home, or certify remote sync.',
      inputSchema: z.object({ entity: z.string().min(1).max(2048) }).strict(),
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
      _meta: { ui: { resourceUri: notebookURI } },
    }, async ({ entity }, context) => {
      try {
        const notebook = await readNotebook(backend, entity, context.signal);
        return { content: [{ type: 'text', text: JSON.stringify(notebook) }], structuredContent: notebook };
      } catch (error) {
        return { isError: true, content: [{ type: 'text', text: error instanceof TonkToolError
          ? error.message : 'Could not read a consistent notebook. Refresh after sync or inspect the notebook in Tonk.' }] };
      }
    });
  }
  server.registerResource('tonk-query', viewURI, { mimeType }, async () => ({
    contents: [{ uri: viewURI, mimeType, text: html,
      _meta: { ui: { prefersBorder: true, csp: { connectDomains: [], resourceDomains: [] } },
        'openai/widgetCSP': { connect_domains: [], resource_domains: [], redirect_domains: ['https://tonk.foundation'] } },
    }],
  }));
  server.registerTool('tonk_show_query', {
    description: 'Display the attached Tonk space query results in an interactive table. Pass the same inline document as tonk_query. Refresh reads the same query; this view does not mutate records or certify synchronization.',
    inputSchema: z.object({ document: z.string().min(1).max(32000) }).strict(),
    annotations: { readOnlyHint: true, openWorldHint: false },
    _meta: { ui: { resourceUri: viewURI } },
  }, async ({ document }, context) => {
    try {
      const result = await backend.call('tonk_query', { document }, context.signal);
      if (!Array.isArray(result?.matches)) throw new Error('Invalid query response.');
      // Only known notebook result shapes receive links. The model cannot
      // supply a destination origin, selected space or URL fragment.
      if (backend.webOrigin && backend.capabilities.includes('tonk_space_info')) {
        const { subject } = await backend.call('tonk_space_info', {}, context.signal);
        for (const block of result.matches) {
          if (!['notebook', 'notebook/named'].includes(block.label)) continue;
          for (const row of block.results ?? []) row.url = notebookURL(backend.webOrigin, subject, row.this);
        }
      }
      return {
        content: [{ type: 'text', text: JSON.stringify(result) }],
        structuredContent: { ...result, document },
      };
    } catch (error) {
      return { isError: true, content: [{ type: 'text', text: error instanceof TonkToolError
        ? error.message : 'Could not read the Tonk space. Reconnect and retry the query.' }] };
    }
  });
}
