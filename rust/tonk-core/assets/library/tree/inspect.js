// Draw the selected node in the right pane: its full hash, size, count,
// rank, the subtree's scale, what it buffers and what its parent buffers
// for it, whether the replica holds it, the manifest it was written
// under, its separator key whole, and for a segment its entries.
(self) => {
  const kit = self.kit();
  const state = self.__tree;
  const pane = state.shadow.querySelector('.inspector');
  if (!pane) return;
  pane.replaceChildren();
  const hash = state.selected;
  if (!hash) return pane.append(kit.el('div', 'empty', 'no node selected'));
  const node = state.nodes.get(hash);
  if (!node) return pane.append(kit.el('div', 'status', 'loading…'));
  const kv = (key, value) => {
    const row = kit.el('div', 'kv');
    row.append(kit.el('span', 'k', key), kit.el('span', 'v', value));
    return row;
  };
  const index = node.kind === 'index';
  pane.append(kit.el('h2', null, index ? 'Index node' : 'Segment node'));
  pane.append(kv('hash', hash.startsWith('#') ? hash.slice(1) : hash));
  const size = kv('size', kit.human(node.size));
  const width = Math.round(Math.max(2, 80 * (node.size / Math.max(1, state.maxSize))));
  const bar = kit.el('span', 'sizebar');
  bar.style.width = `${width}px`;
  size.append(bar);
  pane.append(size, kv(index ? 'children' : 'entries', String(node.count)));
  if (node.rank !== null) pane.append(kv('rank', String(node.rank)));
  if (node.scale !== null) pane.append(kv('scale', String(node.scale)));
  if (node.novelty !== null) pane.append(kv('novelty', String(node.novelty)));
  if (node.pending !== null) pane.append(kv('buffered in parent', String(node.pending)));
  pane.append(kv('storage', node.cached ? 'local' : 'remote (fetched on demand)'));
  if (node.manifest.length > 0) {
    pane.append(kit.el('h3', null, 'manifest'));
    for (const [label, value] of node.manifest) pane.append(kv(label, String(value)));
  }
  if (node.boundParts.length > 0) {
    pane.append(kv('separator', ''));
    const key = kit.el('div', 'keybytes');
    self.key(key, node.boundParts);
    pane.append(key);
  }
  if (!index) self.entries(pane, hash);
}
