// Everything the rows say, gathered: the branches and the roots they name,
// the explorations and what is open in each, and what each open node holds.
(self, rows) => {
  const number = (value) => Number(value ?? 0);
  const text = (value) => (value === undefined || value === null ? '' : String(value));
  const fields = (row) => row?.fields ?? {};
  const group = (map, key, value) => {
    if (!map.has(key)) map.set(key, []);
    map.get(key).push(value);
  };
  const once = (list, id) => {
    const seen = new Set();
    return list.filter((row) => {
      const key = text(id(row));
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  };

  const branches = rows.branch
    .map((row) => ({ branch: text(row.this), name: text(fields(row).name), tree: text(fields(row).tree) }))
    .filter((row) => row.tree.startsWith('node:'));

  const explorations = new Map();
  for (const row of rows.exploration) {
    const root = text(row.this);
    if (root && !explorations.has(root)) explorations.set(root, { root, started: number(fields(row).started), open: new Set() });
  }
  for (const row of rows.opened) {
    const open = fields(row).open;
    for (const node of Array.isArray(open) ? open : [open]) {
      if (node) explorations.get(text(row.this))?.open.add(text(node));
    }
  }

  const nodes = new Map();
  for (const row of rows.node) {
    const f = fields(row);
    const format = {};
    for (const name of ['version', 'fanout-exponent', 'max-separator', 'inline-n', 'spill-prefix', 'max-segment', 'frame-ceiling-factor', 'anchor-selector', 'entry-overhead', 'key-overhead', 'link-overhead']) {
      format[name] = number(f[name]);
    }
    nodes.set(text(row.this), { kind: text(f.kind), size: number(f.size), count: number(f.count), scale: number(f.scale), novelty: number(f.novelty), format });
  }

  // Whether the replica holds a node, as dialog last answered; a node
  // never asked about is in neither.
  const held = new Map();
  for (const row of rows.held) held.set(text(row.this), fields(row).local === true);

  const spills = new Map();
  for (const row of rows.spill) spills.set(text(row.this), text(fields(row).value));

  const values = new Map();
  for (const row of rows.value) {
    const f = fields(row);
    values.set(text(row.this), { size: number(f.size), bytes: self.bytes(f.bytes) });
  }

  const spans = new Map();
  for (const row of once(rows.span, (row) => row.this)) {
    const f = fields(row);
    group(spans, text(f.parent), { span: text(row.this), at: number(f.at), child: text(f.child), separator: text(f.separator), scale: number(f.scale), novelty: number(f.novelty) });
  }
  for (const list of spans.values()) list.sort((a, b) => a.at - b.at);

  const entries = new Map();
  for (const row of once(rows.entry, (row) => row.this)) {
    const f = fields(row);
    group(entries, text(f.segment), { entry: text(row.this), at: number(f.at), key: text(f.key), state: text(f.state), rank: number(f.rank) });
  }
  for (const list of entries.values()) list.sort((a, b) => a.at - b.at);

  // A part is `part:<hash of its key>/<position>`: every part of one key
  // shares the prefix, which is how a key finds its parts.
  const parts = new Map();
  for (const row of once(rows.part, (row) => row.this)) {
    const part = text(row.this);
    const f = fields(row);
    group(parts, part.slice(0, part.lastIndexOf('/')), { at: number(f.at), kind: text(f.kind), text: text(f.text) });
  }
  for (const list of parts.values()) list.sort((a, b) => a.at - b.at);
  const partsOf = (part) => parts.get(part.slice(0, part.lastIndexOf('/'))) ?? null;

  return { branches, explorations, nodes, held, spills, values, spans, entries, partsOf };
}
