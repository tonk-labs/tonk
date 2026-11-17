// Draw the outline and the detail pane from what is known. The whole
// outline is rebuilt: it holds only what has been opened, and focus and
// scroll are put back after.
(self) => {
  const state = self.__tree;
  const { shell, shadow } = state;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const size = (bytes) => {
    const n = Number(bytes ?? 0);
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  };
  // A key or a separator as its parts, each in its kind's color.
  const key = (formula, bytes, empty) => {
    const box = element('span', 'key');
    if (!bytes.length) {
      box.append(element('span', 'part part--min', empty));
      return box;
    }
    const parts = self.label(formula, bytes);
    if (!parts) box.append(element('span', 'part part--pending', `${bytes.length} bytes`));
    else if (!parts.length) box.append(element('span', 'part part--opaque', `${bytes.length} bytes`));
    else {
      for (const part of parts) {
        const piece = element('span', `part part--${part.kind}`, part.text || part.kind);
        piece.title = part.kind;
        box.append(piece);
      }
    }
    return box;
  };

  const focused = shadow.activeElement?.dataset?.id ?? null;
  const previous = shell.querySelector('.outline');
  const scroll = previous ? previous.scrollTop : 0;

  const list = element('div', 'outline');
  list.setAttribute('role', 'tree');
  list.setAttribute('aria-label', 'Search tree');
  const head = element('div', 'head');
  head.setAttribute('aria-hidden', 'true');
  head.append(element('span', 'label', 'key'), element('span', 'col size', 'size'), element('span', 'col count', 'count'), element('span', 'col buffered', 'buffered'));
  list.append(head);

  // One row. `guides` says, for each ancestor level, whether its spine
  // runs on past this row; `last` whether this row ends its own.
  let first = true;
  const row = (id, guides, last, expanded) => {
    const item = element('div', 'row');
    item.setAttribute('role', 'treeitem');
    item.setAttribute('aria-level', String(guides.length + 1));
    item.setAttribute('aria-selected', String(state.selected === id));
    if (expanded !== null) item.setAttribute('aria-expanded', String(expanded));
    item.tabIndex = state.selected === id || (!state.selected && first) ? 0 : -1;
    first = false;
    item.dataset.id = id;
    const indent = element('span', 'indent');
    indent.setAttribute('aria-hidden', 'true');
    guides.forEach((on, level) => {
      // The deepest level is this row's own elbow; the rest carry spines.
      if (level === guides.length - 1) indent.append(element('span', `gutter elbow${last ? ' last' : ''}`));
      else indent.append(element('span', `gutter${on ? ' on' : ''}`));
    });
    item.append(indent);
    list.append(item);
    return item;
  };
  const columns = (item, bytes, widest, count, buffered) => {
    const cell = element('span', 'col size');
    if (bytes !== undefined) {
      const bar = element('span', 'bar');
      bar.style.width = `${Math.max(2, Math.round(70 * Number(bytes) / Math.max(1, widest)))}px`;
      cell.append(bar, element('span', 'num', size(bytes)));
    }
    const buffer = element('span', 'col buffered');
    if (buffered > 0) buffer.append(element('span', 'chip chip--pending', `${buffered} op${buffered === 1 ? '' : 's'}`));
    item.append(cell, element('span', 'col count', count === undefined ? '' : String(count)), buffer);
  };

  const draw = (node, guides, last, span, root, widest) => {
    const known = state.nodes.get(node);
    const open = !!known?.open;
    const item = row(node, guides, last, open);
    item.dataset.node = node;
    const dot = element('span', `dot${known?.info ? ' read' : ''}${known?.loading ? ' loading' : ''}`);
    dot.setAttribute('aria-hidden', 'true');
    item.append(dot);
    if (root) item.append(element('span', 'name', root.name || 'branch'));
    else item.append(key('dialog/separator-part', span.separator, 'start'));
    item.title = `${known?.info?.kind ?? 'node'} ${node}`;
    columns(item, known?.info?.size, widest, known?.info?.count, Number(span?.novelty ?? known?.info?.novelty ?? 0));
    if (!open) return;

    const below = [...guides, !last];
    if (known.error) {
      row(`${node}/error`, [...below, false], true, null).append(element('span', 'note note--error', known.error));
    }
    const spans = known.spans ?? [];
    const sizes = spans.map((child) => Number(state.nodes.get(child.node)?.info?.size ?? 0));
    const widestChild = Math.max(1, ...sizes);
    const entries = (known.entries ?? []).slice(0, known.shown);
    const more = (known.entries?.length ?? 0) > known.shown;
    spans.forEach((child, at) => draw(child.node, below, at === spans.length - 1, child, null, widestChild));
    entries.forEach((entry, at) => {
      const id = String(entry.entry ?? `${node}/${entry.at}`);
      const line = row(id, [...below, false], at === entries.length - 1 && !more, null);
      line.dataset.entry = id;
      line.append(element('span', 'dot leaf'));
      line.append(element('span', 'at', String(entry.at)));
      line.append(key('dialog/key-part', entry.key, 'empty key'));
      const cell = element('span', 'col buffered');
      if (entry.state === 'removed') cell.append(element('span', 'chip chip--removed', 'removed'));
      else if (entry.retraction === true) cell.append(element('span', 'chip chip--removed', 'retracts'));
      else if (entry.spill || entry.blob) cell.append(element('span', 'chip chip--spill', entry.blob ? size(entry['blob-size']) : 'spilled'));
      // An entry has no size or count of its own: its key takes the room.
      line.append(cell);
    });
    if (more) {
      const rest = known.entries.length - known.shown;
      const line = row(`${node}/more`, [...below, false], true, null);
      line.dataset.more = node;
      line.append(element('span', 'link', `show ${Math.min(100, rest)} more of ${rest}`));
    }
  };

  if (state.error) list.append(element('p', 'note note--error', state.error));
  else if (!state.roots.length) list.append(element('p', 'note', 'This branch has no tree yet. The first write to the space makes one.'));
  const widest = Math.max(1, ...state.roots.map((root) => Number(state.nodes.get(root.node)?.info?.size ?? 0)));
  state.roots.forEach((root, at) => draw(root.node, [], at === state.roots.length - 1, null, root, widest));

  shell.replaceChildren(list, self.detail());
  list.scrollTop = scroll;
  if (focused) {
    for (const item of list.querySelectorAll('.row')) {
      if (item.dataset.id === focused) { item.focus({ preventScroll: true }); break; }
    }
  }
}
