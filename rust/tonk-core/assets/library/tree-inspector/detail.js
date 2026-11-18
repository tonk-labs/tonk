// The pane beside the outline: the selected node, what it was written
// under, its separator, and its entries (a segment's, or the operations
// an index node buffers) as a table whose rows unfold.
(self, model) => {
  const state = self.__tree;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const pane = element('aside', 'detail');
  pane.setAttribute('aria-label', 'Selected node');
  const selected = state.selected || self.getAttribute('selected') || '';
  pane.dataset.for = selected;
  const kv = (name, value, extra) => {
    const line = element('div', 'kv');
    line.append(element('span', 'k', name));
    if (value !== '') line.append(element('span', 'v', String(value)));
    if (extra) line.append(extra);
    return line;
  };

  if (!selected.startsWith('node:')) {
    pane.append(element('p', 'note', 'Select a node to see what it holds.'));
    return pane;
  }
  const info = model.nodes.get(selected);
  if (!info) {
    pane.append(element('p', 'note', model.held.get(selected) === false ? 'Not held here: opening it fetches it.' : 'Reading…'));
    return pane;
  }
  // The span that names the node in its parent, if one is open.
  let span = null;
  for (const list of model.spans.values()) {
    span = list.find((candidate) => candidate.child === selected) ?? null;
    if (span) break;
  }
  const held = model.held.get(selected);

  pane.append(element('h2', '', info.kind === 'index' ? 'Index node' : 'Segment node'));
  // The node's entity, which starts an exploration rooted at it.
  const base = (self.getAttribute('base') || '/console/tree').replace(/\/$/, '');
  const link = element('a', 'entity', selected);
  link.href = `${base}/${selected}`;
  link.dataset.explore = selected;
  link.title = 'explore the tree rooted here';
  pane.append(link);
  const bar = element('span', 'bar');
  bar.style.width = `${Math.max(2, Math.round(80 * info.size / Math.max(1, state.widest ?? info.size)))}px`;
  pane.append(kv('size', self.size(info.size), bar));
  pane.append(kv(info.kind === 'index' ? 'children' : 'entries', info.count));
  if (span) pane.append(kv('rank', span.rank));
  pane.append(kv('scale', info.scale));
  pane.append(kv('novelty', info.novelty));
  if (span) pane.append(kv('buffered in parent', span.novelty));
  const storage = element('span', `storage storage--${held === true ? 'local' : held === false ? 'remote' : 'unknown'}`, held === true ? '● local' : held === false ? '○ remote, fetched on open' : '◌ not asked yet');
  pane.append(kv('storage', '', storage));

  // The header a node was written under. Nodes of one tree mostly share
  // it, so it is folded but on the root.
  const header = element('details', 'header');
  state.headers ??= new Map();
  header.open = state.headers.get(selected) ?? selected === self.getAttribute('selected');
  header.addEventListener('toggle', () => state.headers.set(selected, header.open));
  header.append(element('summary', '', 'header'));
  pane.append(header);
  const format = info.format;
  for (const [name, value] of [
    ['version', format.version],
    ['fanout', 2 ** format['fanout-exponent']],
    ['max-separator', format['max-separator']],
    ['inline', format['inline-n']],
    ['spill-prefix', format['spill-prefix']],
    ['max-segment', format['max-segment']],
    ['frame-ceiling', format['frame-ceiling-factor']],
    ['anchor-selector', format['anchor-selector']],
    ['entry-overhead', format['entry-overhead']],
    ['key-overhead', format['key-overhead']],
    ['link-overhead', format['link-overhead']],
  ]) {
    header.append(kv(name, value));
  }

  if (span) {
    pane.append(element('h3', '', 'separator'));
    pane.append(self.chips(model.partsOf(span.separator)));
  }

  const entries = model.entries.get(selected) ?? [];
  const buffered = info.kind === 'index';
  if (buffered && info.novelty === 0) return pane;
  pane.append(element('h3', '', buffered ? `${entries.length} buffered` : `${entries.length} entries`));
  if (!entries.length) {
    pane.append(element('p', 'note', 'Reading…'));
    return pane;
  }
  pane.append(self.entries(model, selected, entries, buffered));
  return pane;
}
