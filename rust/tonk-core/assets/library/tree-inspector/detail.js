// The pane beside the outline: what the session holds of the selected
// node or entry.
(self, model) => {
  const state = self.__tree;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const pane = element('aside', 'detail');
  pane.setAttribute('aria-label', 'Selected item');
  const table = (rows) => {
    const list = element('dl', 'facts');
    for (const [name, value] of rows) {
      if (value === undefined || value === null || value === '') continue;
      list.append(element('dt', '', name));
      list.append(element('dd', '', String(value)));
    }
    return list;
  };
  const chips = (part) => {
    const box = element('div', 'chips');
    for (const piece of model.partsOf(part) ?? []) {
      const span = element('span', `part part--${piece.kind}`, piece.text || piece.kind);
      span.title = piece.kind;
      box.append(span);
    }
    return box;
  };

  const selected = state.selected;
  if (!selected) {
    pane.append(element('p', 'note', 'Select a node or an entry to see what it holds.'));
    return pane;
  }
  const info = model.nodes.get(selected);
  if (info || selected.startsWith('node:')) {
    pane.append(element('h2', '', info?.kind ?? 'node'));
    pane.append(element('span', 'entity', selected));
    if (!info) pane.append(element('p', 'note', 'Open the node to read it.'));
    else {
      const held = model.held.get(selected);
      pane.append(table([
        ['block size', `${info.size} bytes`],
        [info.kind === 'index' ? 'spans' : 'entries', info.count],
        ['scale', info.scale],
        ['buffered ops', info.novelty],
        ['held here', held === undefined ? 'not asked' : held ? 'yes' : 'no'],
      ]));
      pane.append(element('h3', '', 'written under'));
      pane.append(table([
        ['layout version', info.format.version],
        ['fanout exponent', info.format['fanout-exponent']],
        ['max separator', info.format['max-separator']],
        ['inline up to', info.format['inline-n']],
        ['spill prefix', info.format['spill-prefix']],
        ['max segment', info.format['max-segment']],
        ['frame ceiling factor', info.format['frame-ceiling-factor']],
        ['anchor selector', info.format['anchor-selector']],
        ['entry overhead', info.format['entry-overhead']],
        ['key overhead', info.format['key-overhead']],
        ['link overhead', info.format['link-overhead']],
      ]));
    }
    return pane;
  }
  let entry = null;
  for (const list of model.entries.values()) {
    entry = list.find((candidate) => candidate.entry === selected);
    if (entry) break;
  }
  if (!entry) {
    pane.append(element('p', 'note', 'That item is no longer open.'));
    return pane;
  }
  pane.append(element('h2', '', 'entry'));
  pane.append(element('span', 'entity', selected));
  pane.append(element('h3', '', 'key'));
  pane.append(chips(entry.key));
  pane.append(table([
    ['position', entry.at],
    ['state', entry.state],
    ['rank', entry.rank],
  ]));
  const spill = model.spills.get(entry.entry);
  if (spill) {
    pane.append(element('h3', '', 'spilled value'));
    pane.append(element('span', 'entity', spill));
    const value = model.values.get(spill);
    if (!value) {
      // Read only when asked: a value can be large, and may be fetched.
      const read = element('button', 'button', state.pending.get(spill) === 'open' ? 'reading…' : 'read value');
      read.type = 'button';
      read.dataset.value = spill;
      pane.append(read);
    } else {
      const head = Uint8Array.from(value.bytes.slice(0, 4096));
      let shown;
      try {
        shown = new TextDecoder('utf-8', { fatal: true }).decode(head);
      } catch {
        shown = value.bytes.slice(0, 1024).map((byte) => byte.toString(16).padStart(2, '0')).join(' ');
      }
      if (value.bytes.length > head.length) shown += '\n…';
      pane.append(table([['size', `${value.size} bytes`]]));
      pane.append(element('pre', 'value', shown));
    }
  }
  return pane;
}
