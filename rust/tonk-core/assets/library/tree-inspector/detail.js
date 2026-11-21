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
  // A fact of the node: its name, with what it means dim beneath, and
  // its value.
  const kv = (name, value, extra, about) => {
    const line = element('div', 'kv');
    const key = element('div', 'k');
    key.append(element('span', 'name', name));
    if (about) key.append(element('span', 'about', about));
    const shown = element('div', 'v');
    if (value !== '') shown.append(element('span', '', String(value)));
    if (extra) shown.append(extra);
    line.append(key, shown);
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
  const facts = element('div', 'facts');
  pane.append(facts);
  facts.append(kv('size', self.size(info.size), bar, 'The block as stored: what a fetch pays for.'));
  facts.append(info.kind === 'index'
    ? kv('children', info.count, null, 'Spans it delegates, one child node each.')
    : kv('entries', info.count, null, 'Entries it stores.'));
  if (span) facts.append(kv('rank', span.rank, null, 'The level coin of its separator, which made the boundary.'));
  facts.append(kv('scale', info.scale, null, 'An estimate of its subtree’s size, on a log scale.'));
  facts.append(kv('novelty', info.novelty, null, 'Operations it buffers for its children, not written down there yet.'));
  if (span) facts.append(kv('novelty in parent', span.novelty, null, 'Operations its parent buffers against it.'));
  const storage = element('span', `storage storage--${held === true ? 'local' : held === false ? 'remote' : 'unknown'}`, held === true ? '● local' : held === false ? '○ remote' : '◌ not asked yet');
  facts.append(kv('storage', '', storage, 'Whether this replica holds the block; a remote one is fetched when opened.'));

  // The header a node was written under. Nodes of one tree mostly share
  // it, so it is folded but on the root.
  const header = element('details', 'header');
  state.headers ??= new Map();
  header.open = state.headers.get(selected) ?? selected === self.getAttribute('selected');
  header.addEventListener('toggle', () => state.headers.set(selected, header.open));
  header.append(element('summary', '', 'Header'));
  const written = element('div', 'facts');
  header.append(written);
  pane.append(header);
  const format = info.format;
  for (const [name, value, about] of [
    ['version', format.version, 'The layout version, which says how the rest is read.'],
    ['fanout', 2 ** format['fanout-exponent'], 'Children an index node is expected to have.'],
    ['max-separator', format['max-separator'], 'Keys longer than this never become boundaries.'],
    ['inline', format['inline-n'], 'Values longer than this spill out of the key into a blob.'],
    ['spill-prefix', format['spill-prefix'], 'Leading bytes a spilled value’s key keeps, so it still sorts.'],
    ['max-segment', format['max-segment'], 'The weight a segment is cut at; 0 for none.'],
    ['frame-ceiling', format['frame-ceiling-factor'], 'The hard weight ceiling, as a multiple of max-segment.'],
    ['anchor-selector', format['anchor-selector'], 'Which candidate boundary a forced cut lands on.'],
    ['entry-overhead', format['entry-overhead'], 'Weight an entry costs beyond its key and value.'],
    ['key-overhead', format['key-overhead'], 'Weight a key costs beyond its bytes, for the cut floor.'],
    ['link-overhead', format['link-overhead'], 'Weight a child link costs beyond its separator and hash.'],
  ]) {
    written.append(kv(name, value, null, about));
  }

  if (span) {
    pane.append(element('h3', '', 'Separator'));
    pane.append(self.chips(model.partsOf(span.separator)));
  }

  const entries = model.entries.get(selected) ?? [];
  const buffered = info.kind === 'index';
  if (buffered && info.novelty === 0) return pane;
  pane.append(element('h3', '', buffered ? `Novelty (${entries.length})` : `Entries (${entries.length})`));
  if (!entries.length) {
    pane.append(element('p', 'note', 'Reading…'));
    return pane;
  }
  pane.append(self.entries(model, selected, entries, buffered));
  return pane;
}
