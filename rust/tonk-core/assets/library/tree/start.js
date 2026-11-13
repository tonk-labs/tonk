// Build the two panes for this element's route and load the root, which
// starts selected. Nothing to do when the route is the one already shown.
async (self) => {
  const kit = self.kit();
  const route = self.route();
  const key = route?.key ?? null;
  if (self.__tree && self.__tree.key === key) return;
  const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open' });
  if (!route) {
    self.__tree = { key: null };
    shadow.replaceChildren(
      kit.el('style', null, self.stylesheet()),
      kit.el('div', 'err', 'no repository in context (set with="branch@repo", or repo and branch)'),
    );
    return;
  }
  const state = (self.__tree = {
    key, route: route.detail, shadow,
    nodes: new Map(), children: new Map(), root: null, selected: null, maxSize: 1, seq: 0,
  });

  // A shadow root is out of the Web Awesome autoloader's sight, so the tree
  // modules are imported here where the page has not registered them. In a
  // sealed guest they are registered already.
  if (!customElements.get('wa-tree-item')) {
    for (const module of ['tree/tree.js', 'tree-item/tree-item.js', 'tooltip/tooltip.js']) {
      try {
        await import(`/webawesome/components/${module}`);
      } catch {
        // Not served here: the element still renders, unstyled.
      }
    }
  }
  if (self.__tree !== state) return;

  const outline = kit.el('div', 'outline');
  const header = kit.el('div', 'col-header');
  header.append(
    kit.el('span', 'keystr', 'node'),
    kit.el('span', 'col size', 'size'),
    kit.el('span', 'col count', 'count'),
    kit.el('span', 'col novelty', 'novelty'),
  );
  const tree = kit.el('wa-tree');
  tree.setAttribute('selection', 'single');
  outline.append(header, tree);
  const left = kit.el('div', 'pane left');
  left.append(outline);
  const right = kit.el('div', 'pane right');
  right.append(kit.el('div', 'inspector'));
  shadow.replaceChildren(kit.el('style', null, self.stylesheet()), left, right);

  tree.addEventListener('wa-selection-change', (event) => {
    const hash = event.detail?.selection?.[0]?.getAttribute?.('data-hash');
    if (!hash || self.__tree !== state) return;
    state.selected = hash;
    self.inspect();
  });

  try {
    const rows = await self.query('tree/node', {});
    if (self.__tree !== state) return;
    if (rows.length === 0) return self.outline('empty tree');
    const node = self.node(rows[0]);
    state.root = node.hash;
    state.selected = node.hash;
    kit.put(state, node);
    self.outline();
    self.inspect();
  } catch (error) {
    if (self.__tree === state) self.outline(error?.message ?? String(error));
  }
}
