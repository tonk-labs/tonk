// Draw the page from the rows: with no exploration selected, the
// branch's current tree and the explorations of this session; with one,
// its outline and the selected item's detail.
(self) => {
  const state = self.__tree;
  const model = state.model;
  if (!model) {
    state.shell.replaceChildren(Object.assign(document.createElement('p'), { className: state.error ? 'note note--error' : 'note', textContent: state.error ?? 'Reading…' }));
    return;
  }
  const base = (self.getAttribute('base') || '/console/tree').replace(/\/$/, '');
  const branchName = self.getAttribute('branch') || 'main';
  const wanted = self.getAttribute('selected') || '';
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
  // The roots this branch names now, one per replica that has it.
  const current = [...new Set(model.branches.filter((row) => row.name === branchName).map((row) => row.tree))];
  // A key's parts as colored text, or its size while they are read.
  const key = (part, empty) => {
    const box = element('span', 'key');
    const parts = part ? model.partsOf(part) : null;
    if (!part) box.append(element('span', 'part part--min', empty));
    else if (!parts) box.append(element('span', 'part part--pending', '…'));
    else {
      for (const piece of parts) {
        const span = element('span', `part part--${piece.kind}`, piece.text || piece.kind);
        span.title = piece.kind;
        box.append(span);
      }
    }
    return box;
  };
  const explore = (root, label) => {
    const link = element('a', 'button', label);
    link.href = `${base}/${root}`;
    link.dataset.explore = root;
    return link;
  };

  const focused = state.shadow.activeElement?.dataset?.id ?? null;
  const previous = state.shell.querySelector('.outline');
  const scroll = previous ? previous.scrollTop : 0;

  const exploration = model.explorations.get(wanted);
  if (!wanted) {
    state.shell.replaceChildren(self.overview(model, current, base, explore, short));
    return;
  }
  // A root named in the path with no exploration yet: start one.
  if (!exploration && !state.asked.has(wanted)) {
    state.asked.add(wanted);
    self.send('tree-explore', { root: wanted });
  }

  const list = element('div', 'outline');
  list.setAttribute('role', 'tree');
  list.setAttribute('aria-label', 'Search tree');
  const top = element('div', 'toolbar');
  const back = element('a', 'link', '← explorations');
  back.href = base;
  top.append(back);
  if (current.length && !current.includes(wanted)) {
    top.append(element('span', 'chip chip--pending', `${branchName} has moved`));
    for (const root of current) top.append(explore(root, `explore ${short(root)}`));
  }
  list.append(top);
  const head = element('div', 'head');
  head.setAttribute('aria-hidden', 'true');
  head.append(element('span', 'label', 'key'), element('span', 'col size', 'size'), element('span', 'col count', 'count'), element('span', 'col buffered', 'buffered'));
  list.append(head);

  const open = exploration?.open ?? new Set();
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

  const draw = (node, guides, last, span, widest) => {
    const info = model.nodes.get(node);
    const isOpen = open.has(node);
    // Asked and not answered yet: the session has not taken the command.
    const waiting = state.pending.get(node);
    const loading = (waiting === 'open' && !isOpen) || (isOpen && !info);
    if (waiting === 'open' && isOpen) state.pending.delete(node);
    if (waiting === 'close' && !isOpen) state.pending.delete(node);
    const item = row(node, guides, last, isOpen);
    item.dataset.node = node;
    // Filled when the replica holds the node, hollow when it does not,
    // faint until dialog has answered.
    const held = model.held.get(node);
    const dot = element('span', `dot${held === true ? ' read' : held === false ? ' remote' : ' unknown'}${loading ? ' loading' : ''}`);
    dot.title = held === true ? 'held by this replica' : held === false ? 'not held: opening fetches it' : 'not asked yet';
    dot.setAttribute('aria-hidden', 'true');
    item.append(dot);
    if (!span) item.append(element('span', 'name', short(node)));
    else item.append(key(span.separator, 'start'));
    item.title = `${info?.kind ?? 'node'} ${node}`;
    columns(item, info?.size, widest, info?.count, Number(span?.novelty ?? info?.novelty ?? 0));
    if (!isOpen) return;

    const below = [...guides, !last];
    const children = model.spans.get(node) ?? [];
    const sizes = children.map((child) => Number(model.nodes.get(child.child)?.size ?? 0));
    const widestChild = Math.max(1, ...sizes);
    children.forEach((child, at) => draw(child.child, below, at === children.length - 1, child, widestChild));
    const entries = model.entries.get(node) ?? [];
    const shown = state.shown?.get(node) ?? 100;
    const visible = entries.slice(0, shown);
    const more = entries.length > shown;
    visible.forEach((entry, at) => {
      const line = row(entry.entry, [...below, false], at === visible.length - 1 && !more, null);
      line.dataset.entry = entry.entry;
      line.append(element('span', 'dot leaf'));
      line.append(element('span', 'at', String(entry.at)));
      line.append(key(entry.key, 'empty key'));
      const cell = element('span', 'col buffered');
      if (entry.state === 'removed') cell.append(element('span', 'chip chip--removed', 'removed'));
      else if (model.spills.has(entry.entry)) cell.append(element('span', 'chip chip--spill', 'spilled'));
      line.append(cell);
    });
    if (more) {
      const rest = entries.length - shown;
      const line = row(`${node}/more`, [...below, false], true, null);
      line.dataset.more = node;
      line.append(element('span', 'link', `show ${Math.min(100, rest)} more of ${rest}`));
    }
  };

  if (!exploration) list.append(element('p', 'note', 'Starting the exploration…'));
  else draw(wanted, [], true, null, Math.max(1, model.nodes.get(wanted)?.size ?? 1));

  state.model = model;
  state.shell.replaceChildren(list, self.detail(model));
  list.scrollTop = scroll;
  if (focused) {
    for (const item of list.querySelectorAll('.row')) {
      if (item.dataset.id === focused) { item.focus({ preventScroll: true }); break; }
    }
  }
}
