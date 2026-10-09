import { TonkToolError } from './core.mjs';

// Deliberately narrower than every possible URI. These values enter both
// notation and a route, never a caller-selected origin or credential URL.
export function isEntity(value) {
  return typeof value === 'string' && value.length <= 2048 &&
    /^[A-Za-z][A-Za-z0-9+.-]*:[A-Za-z0-9._~:/%+-]+$/.test(value);
}

export function notebookURL(origin, subject, entity) {
  if (!origin || !/^did:key:[A-Za-z0-9]+$/.test(subject) || !isEntity(entity)) return undefined;
  const base = new URL(origin);
  if (base.protocol !== 'https:' || base.origin !== origin) throw new Error('Invalid Tonk web origin.');
  return `${base.origin}/space/${encodeURIComponent(subject)}/notebook/${encodeURIComponent(entity)}`;
}

export async function readNotebook(backend, entity, signal) {
  if (!isEntity(entity)) throw new TonkToolError('Provide the exact notebook entity URI returned by a query.');
  const reference = entity; // The validated URI must remain bare in Tonk notation.
  // One query response is one replica snapshot. Read the title independently
  // so an empty notebook is distinguishable from a missing notebook.
  const document = `notebook/named:\n  this: ${reference}\nnotebook:\n  this: ${reference}\nnotebook/block:\n  notebook: ${reference}\n`;
  const result = await backend.call('tonk_query', { document }, signal);
  if (!Array.isArray(result?.matches) || result.matches.length !== 3) throw new Error('Invalid notebook query response.');
  const [named, placed, contents] = result.matches.map(block => block.results);
  if (![named, placed, contents].every(Array.isArray)) throw new Error('Invalid notebook rows.');
  const titles = new Set(named.filter(row => row.this === entity).map(row => row.fields?.title));
  if (!titles.size) throw new TonkToolError('Notebook not found in the attached space. Query notebook/named to find its entity.');
  if (titles.size !== 1 || typeof [...titles][0] !== 'string') throw new TonkToolError('Notebook title is ambiguous; resolve it in Tonk first.');
  const order = new Map(), sources = new Map();
  for (const row of placed) {
    if (row.this !== entity || !row.fields?.block || typeof row.fields.block !== 'object' || Array.isArray(row.fields.block)) throw new Error('Invalid notebook sequence.');
    for (const [position, block] of Object.entries(row.fields.block)) {
      if (!position || !isEntity(block) || (order.has(position) && order.get(position) !== block)) throw new Error('Ambiguous notebook sequence.');
      order.set(position, block);
    }
  }
  for (const row of contents) {
    if (row.fields?.notebook !== entity || typeof row.fields?.source !== 'string' || !isEntity(row.this)) throw new Error('Invalid notebook block.');
    if (sources.has(row.this) && sources.get(row.this) !== row.fields.source) throw new Error('Ambiguous notebook block.');
    sources.set(row.this, row.fields.source);
  }
  const blocks = [...order].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([position, id]) => {
    if (!sources.has(id)) throw new TonkToolError('A placed notebook block is missing. Refresh after sync; no partial notebook was returned.');
    return { entity: id, position, source: sources.get(id) };
  });
  const selected = new Set(blocks.map(block => block.entity));
  const unplacedBlocks = [...sources.keys()].filter(id => !selected.has(id)).length;
  const { subject } = await backend.call('tonk_space_info', {}, signal);
  return { entity, title: [...titles][0], blocks, unplacedBlocks,
    markdown: blocks.map(block => block.source).join('\n\n'), revision: result.revision,
    url: notebookURL(backend.webOrigin, subject, entity),
    readOnly: true, scope: 'Read-only notebook snapshot from the attached replica. Refresh reads again. Live query cells are displayed as source, not executed.' };
}
