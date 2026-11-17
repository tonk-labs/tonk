// Everything the rows say, gathered: the branches and the roots they name,
// the explorations and what is open in each, and what each open node holds.
(self) => {
  const data = self.querySelector(':scope > .ti-data');
  const rows = (selector) => (data ? [...data.querySelectorAll(selector)] : []);
  const number = (value) => Number(value ?? 0);
  const group = (map, key, value) => {
    if (!map.has(key)) map.set(key, []);
    map.get(key).push(value);
  };

  const branches = rows('.ti-branch-row')
    .map((row) => ({ branch: row.dataset.branch, name: row.dataset.name ?? '', tree: row.dataset.tree ?? '' }))
    .filter((row) => row.tree.startsWith('node:'));

  const explorations = new Map();
  for (const row of rows('.ti-exploration-row')) {
    const root = row.dataset.root;
    if (root && !explorations.has(root)) explorations.set(root, { root, started: number(row.dataset.started), open: new Set() });
  }
  for (const row of rows('.ti-opened-row')) {
    explorations.get(row.dataset.exploration)?.open.add(row.dataset.node);
  }

  const nodes = new Map();
  for (const row of rows('.ti-node-row')) {
    nodes.set(row.dataset.node, {
      kind: row.dataset.kind,
      size: number(row.dataset.size),
      count: number(row.dataset.count),
      scale: number(row.dataset.scale),
      novelty: number(row.dataset.novelty),
    });
  }

  const spans = new Map();
  const seenSpans = new Set();
  for (const row of rows('.ti-span-row')) {
    if (seenSpans.has(row.dataset.span)) continue;
    seenSpans.add(row.dataset.span);
    group(spans, row.dataset.parent, {
      span: row.dataset.span,
      at: number(row.dataset.at),
      child: row.dataset.child,
      separator: row.dataset.separator ?? '',
      scale: number(row.dataset.scale),
      novelty: number(row.dataset.novelty),
    });
  }
  for (const list of spans.values()) list.sort((a, b) => a.at - b.at);

  const entries = new Map();
  const seenEntries = new Set();
  for (const row of rows('.ti-entry-row')) {
    if (seenEntries.has(row.dataset.entry)) continue;
    seenEntries.add(row.dataset.entry);
    group(entries, row.dataset.segment, {
      entry: row.dataset.entry,
      at: number(row.dataset.at),
      key: row.dataset.key ?? '',
      state: row.dataset.state ?? '',
      rank: number(row.dataset.rank),
    });
  }
  for (const list of entries.values()) list.sort((a, b) => a.at - b.at);

  // A part is `part:<hash of its key>/<position>`: every part of one key
  // shares the prefix, which is how a key finds its parts.
  const parts = new Map();
  const seenParts = new Set();
  for (const row of rows('.ti-part-row')) {
    const part = row.dataset.part ?? '';
    if (seenParts.has(part)) continue;
    seenParts.add(part);
    group(parts, part.slice(0, part.lastIndexOf('/')), {
      at: number(row.dataset.at),
      kind: row.dataset.kind ?? '',
      text: row.dataset.text ?? '',
    });
  }
  for (const list of parts.values()) list.sort((a, b) => a.at - b.at);
  const partsOf = (part) => parts.get(part.slice(0, part.lastIndexOf('/'))) ?? null;

  return { branches, explorations, nodes, spans, entries, partsOf };
}
