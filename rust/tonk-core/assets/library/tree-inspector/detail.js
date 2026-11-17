// The pane beside the outline: everything dialog says of the selected
// node or entry.
(self) => {
  const state = self.__tree;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const pane = element('aside', 'detail');
  pane.setAttribute('aria-label', 'Selected item');
  const hex = (bytes) => bytes.map((byte) => byte.toString(16).padStart(2, '0')).join('');
  const table = (rows) => {
    const list = element('dl', 'facts');
    for (const [name, value] of rows) {
      if (value === undefined || value === null || value === '') continue;
      list.append(element('dt', '', name));
      const cell = element('dd');
      if (value instanceof Node) cell.append(value);
      else cell.textContent = String(value);
      list.append(cell);
    }
    return list;
  };
  const parts = (formula, bytes) => {
    const decoded = self.label(formula, bytes) ?? [];
    const box = element('div', 'chips');
    for (const part of decoded) {
      const piece = element('span', `part part--${part.kind}`, part.text || part.kind);
      piece.title = part.kind;
      box.append(piece);
    }
    return box;
  };

  const selected = state.selected;
  if (!selected) {
    pane.append(element('p', 'note', 'Select a node or an entry to see what it holds.'));
    return pane;
  }

  const known = state.nodes.get(selected);
  if (known) {
    pane.append(element('h2', '', known.info?.kind === 'segment' ? 'Segment' : known.info?.kind === 'index' ? 'Index' : 'Node'));
    pane.append(element('span', 'entity', selected));
    const info = known.info ?? {};
    pane.append(table([
      ['kind', info.kind],
      ['block size', info.size !== undefined ? `${info.size} bytes` : ''],
      [info.kind === 'index' ? 'spans' : 'entries', info.count],
      ['scale', info.scale],
      ['buffered ops', info.novelty],
    ]));
    if (known.info) {
      pane.append(element('h3', '', 'Written under'));
      pane.append(table([
        ['layout version', info.version],
        ['fanout exponent', info['fanout-exponent']],
        ['max separator', info['max-separator']],
        ['inline up to', info['inline-n']],
        ['spill prefix', info['spill-prefix']],
        ['max segment', info['max-segment']],
        ['frame ceiling factor', info['frame-ceiling-factor']],
        ['anchor selector', info['anchor-selector']],
        ['entry overhead', info['entry-overhead']],
        ['key overhead', info['key-overhead']],
        ['link overhead', info['link-overhead']],
      ]));
    } else if (!known.loading) {
      pane.append(element('p', 'note', 'Open the node to read it.'));
    }
    return pane;
  }

  // An entry: found in the segment it was drawn under.
  let entry = null;
  for (const node of state.nodes.values()) {
    entry = (node.entries ?? []).find((candidate) => String(candidate.entry ?? `${node.node}/${candidate.at}`) === selected);
    if (entry) break;
  }
  if (!entry) {
    pane.append(element('p', 'note', 'That item is no longer in the tree.'));
    return pane;
  }
  pane.append(element('h2', '', 'Entry'));
  pane.append(element('span', 'entity', selected));
  pane.append(element('h3', '', 'Key'));
  pane.append(parts('dialog/key-part', entry.key));
  pane.append(table([
    ['position', entry.at],
    ['state', entry.state],
    ['retraction', entry.retraction === undefined ? '' : String(entry.retraction)],
    ['rank', entry.rank],
    ['edition', entry.edition],
    ['origin', entry.origin ? element('code', '', hex(self.bytes(entry.origin))) : ''],
    ['cause', entry.cause],
    ['collapsed', entry.collapsed],
    ['supersedes', entry.supersedes],
    ['blob', entry.blob ? element('code', '', String(entry.blob)) : ''],
    ['blob size', entry['blob-size'] !== undefined ? `${entry['blob-size']} bytes` : ''],
    ['blob version', entry['blob-version']],
    ['key bytes', element('code', 'wrap', hex(entry.key))],
  ]));
  if (entry.spill) {
    pane.append(element('h3', '', 'Spilled value'));
    pane.append(element('span', 'entity', String(entry.spill)));
    const value = entry.value;
    if (!value) {
      const read = element('button', 'button', 'Read value');
      read.type = 'button';
      read.dataset.value = selected;
      pane.append(read);
    } else if (value.error) {
      pane.append(element('p', 'note note--error', value.error));
    } else if (value.loading) {
      pane.append(element('p', 'note', 'Reading…'));
    } else {
      pane.append(table([['size', `${value.size} bytes`]]));
      pane.append(element('pre', 'value', value.text));
    }
  }
  return pane;
}
