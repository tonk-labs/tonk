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
  const short = (entity) => {
    const id = String(entity).replace(/^[a-z]+:/, '');
    return id.length > 12 ? `${id.slice(0, 6)}…${id.slice(-4)}` : id;
  };
  // A key or a separator as its components, each in its kind's color.
  const key = (formula, bytes, empty) => {
    const box = element('span', 'key');
    if (!bytes.length) {
      box.append(element('span', 'part part--min', empty));
      return box;
    }
    const parts = self.label(formula, bytes);
    if (!parts) {
      box.append(element('span', 'part part--pending', `${bytes.length} bytes`));
    } else if (!parts.length) {
      box.append(element('span', 'part part--opaque', `${bytes.length} bytes`));
    } else {
      for (const part of parts) {
        const piece = element('span', `part part--${part.kind}`, part.text || part.kind);
        piece.title = part.kind;
        box.append(piece);
      }
    }
    return box;
  };

  const focused = shadow.activeElement?.dataset?.id ?? null;
  const outline = shell.querySelector('.outline');
  const scroll = outline ? outline.scrollTop : 0;

  const list = element('div', 'outline');
  list.setAttribute('role', 'tree');
  list.setAttribute('aria-label', 'Search tree');

  const row = (id, depth, expanded) => {
    const item = element('div', 'row');
    item.setAttribute('role', 'treeitem');
    item.setAttribute('aria-level', String(depth + 1));
    item.setAttribute('aria-selected', String(state.selected === id));
    if (expanded !== null) item.setAttribute('aria-expanded', String(expanded));
    item.style.setProperty('--depth', String(depth));
    item.tabIndex = state.selected === id || (!state.selected && !list.firstChild) ? 0 : -1;
    item.dataset.id = id;
    list.append(item);
    return item;
  };

  const draw = (node, depth, span, root) => {
    const known = state.nodes.get(node);
    const open = !!known?.open;
    const item = row(node, depth, open);
    item.dataset.node = node;
    const twist = element('span', 'twist', open ? '▾' : '▸');
    twist.setAttribute('aria-hidden', 'true');
    item.append(twist);
    const kind = known?.info?.kind;
    item.append(element('span', `badge badge--${kind ?? 'unread'}`, kind ?? 'node'));
    if (root) item.append(element('span', 'name', root.name || 'branch'));
    else item.append(key('dialog/separator-part', span.separator, 'start'));
    const meta = element('span', 'meta');
    if (known?.info) {
      const count = Number(known.info.count ?? 0);
      meta.append(element('span', '', `${count} ${kind === 'index' ? 'span' : 'entr'}${kind === 'index' ? (count === 1 ? '' : 's') : (count === 1 ? 'y' : 'ies')}`));
      meta.append(element('span', '', size(known.info.size)));
    }
    const novelty = Number(span?.novelty ?? known?.info?.novelty ?? 0);
    if (novelty > 0) meta.append(element('span', 'pending', `${novelty} buffered`));
    meta.append(element('code', 'hash', short(node)));
    item.append(meta);
    if (!open) return;

    if (known.loading) {
      row(`${node}/loading`, depth + 1, null).append(element('span', 'note', 'Reading…'));
    }
    if (known.error) {
      row(`${node}/error`, depth + 1, null).append(element('span', 'note note--error', known.error));
    }
    for (const child of known.spans ?? []) draw(child.node, depth + 1, child, null);
    const entries = known.entries ?? [];
    for (const entry of entries.slice(0, known.shown)) {
      const id = String(entry.entry ?? `${node}/${entry.at}`);
      const line = row(id, depth + 1, null);
      line.dataset.entry = id;
      line.dataset.of = node;
      line.dataset.at = String(entry.at);
      line.append(element('span', 'twist', ''));
      line.append(element('span', 'at', String(entry.at)));
      line.append(key('dialog/key-part', entry.key, 'empty key'));
      const meta = element('span', 'meta');
      if (entry.state === 'removed') meta.append(element('span', 'badge badge--removed', 'removed'));
      if (entry.retraction === true) meta.append(element('span', 'badge badge--removed', 'retraction'));
      if (entry.spill) meta.append(element('span', 'badge badge--spill', 'spilled'));
      if (entry.blob) meta.append(element('span', 'badge badge--spill', size(entry['blob-size'])));
      line.append(meta);
    }
    if (entries.length > known.shown) {
      const more = row(`${node}/more`, depth + 1, null);
      more.dataset.more = node;
      more.append(element('span', 'twist', ''));
      more.append(element('span', 'link', `Show ${Math.min(100, entries.length - known.shown)} more of ${entries.length - known.shown}`));
    }
  };

  if (state.error) list.append(element('p', 'note note--error', state.error));
  else if (!state.roots.length) list.append(element('p', 'note', 'This branch has no tree yet. The first write to the space makes one.'));
  for (const root of state.roots) draw(root.node, 0, null, root);

  shell.replaceChildren(list, self.detail());
  list.scrollTop = scroll;
  if (focused) {
    for (const item of list.children) {
      if (item.dataset.id === focused) { item.focus({ preventScroll: true }); break; }
    }
  }
}
