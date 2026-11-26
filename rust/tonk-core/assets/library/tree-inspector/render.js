// Draw the page from the rows: with no exploration selected, the
// branch's current tree and the explorations of this session; with one,
// its outline of nodes and the selected node's detail.
(self) => {
  const state = self.__tree;
  // The element's methods are replaced live when the component is
  // upgraded, but the stylesheet is written once: write it again when the
  // `sheet` method has changed.
  const style = state.style ?? state.shadow.querySelector('style');
  const sheet = self.sheet();
  if (style && style.textContent !== sheet) style.textContent = sheet;
  state.style = style;
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
  const short = (entity) => {
    const id = String(entity).replace(/^[a-z]+:/, '');
    return id.length > 12 ? `${id.slice(0, 6)}…${id.slice(-4)}` : id;
  };
  // The roots this branch names now, one per replica that has it.
  const current = [...new Set(model.branches.filter((row) => row.name === branchName).map((row) => row.tree))];
  const explore = (root, label) => {
    const link = element('a', 'button', label);
    link.href = `${base}/${root}`;
    link.dataset.explore = root;
    return link;
  };

  const focused = state.shadow.activeElement?.dataset?.id ?? null;
  const previous = state.shell.querySelector('.outline');
  const scroll = previous ? previous.scrollTop : 0;
  const pane = state.shell.querySelector('.detail');
  const paneScroll = pane ? pane.scrollTop : 0;

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
  const legend = element('span', 'legend', '● local  ○ remote  ◌ not asked');
  legend.title = 'whether this replica holds a node: a remote one is fetched when opened';
  head.append(element('span', 'label', 'node'), legend, element('span', 'col size', 'size'), element('span', 'col count', 'count'), element('span', 'col novelty', 'novelty'), element('span', 'cloud'));
  list.append(head);

  // The widest node shown sets the bars' scale, as one ruler for the page.
  let widest = 1;
  for (const info of model.nodes.values()) widest = Math.max(widest, info.size);
  state.widest = widest;

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

  const draw = (node, guides, last, span, neighbors) => {
    const info = model.nodes.get(node);
    const isOpen = open.has(node);
    const index = info?.kind === 'index';
    // Asked and not answered yet: the session has not taken the command.
    const waiting = state.pending.get(node);
    const loading = (waiting === 'open' && !isOpen) || (isOpen && !info);
    if (waiting === 'open' && isOpen) state.pending.delete(node);
    if (waiting === 'close' && !isOpen) state.pending.delete(node);
    const held = model.held.get(node);
    const item = row(node, guides, last, index || !info ? isOpen && index : null);
    item.dataset.node = node;
    if (held === false) item.classList.add('remote');
    // Filled when the replica holds the node, hollow when it does not,
    // faint until dialog has answered.
    const dot = element('span', `dot${held === true ? ' read' : held === false ? ' remote' : ' unknown'}${loading ? ' loading' : ''}`);
    dot.title = held === true ? 'held by this replica' : held === false ? 'not held: opening fetches it' : 'not asked yet';
    dot.setAttribute('aria-hidden', 'true');
    item.append(dot);
    const label = element('span', 'keystr');
    if (!span) label.append(element('span', 'name', short(node)));
    else label.append(self.key(model.partsOf(span.separator), neighbors));
    item.append(label);
    item.title = `${info?.kind ?? 'node'} ${node}`;
    item.append(self.columns(info, span, held));
    if (!isOpen || !index) return;

    const below = [...guides, !last];
    const children = model.spans.get(node) ?? [];
    const parts = children.map((child) => model.partsOf(child.separator));
    const own = span ? model.partsOf(span.separator) : null;
    children.forEach((child, at) =>
      draw(child.child, below, at === children.length - 1, child, { previous: at === 0 ? own : parts[at - 1], next: parts[at + 1] ?? null }),
    );
  };

  if (!exploration) list.append(element('p', 'note', 'Starting the exploration…'));
  else draw(wanted, [], true, null, null);

  state.model = model;
  const detail = self.detail(model);
  state.shell.replaceChildren(list, detail);
  list.scrollTop = scroll;
  if (pane && detail.dataset.for === pane.dataset.for) detail.scrollTop = paneScroll;
  if (focused) {
    for (const item of list.querySelectorAll('.row')) {
      if (item.dataset.id === focused) { item.focus({ preventScroll: true }); break; }
    }
  }
}
