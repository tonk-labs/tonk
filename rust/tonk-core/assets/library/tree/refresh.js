// Read the node `hash` again, now that expanding fetched it, and redraw its
// row in place: the dot fills, the cloud goes, the numbers are real.
//
// The read describes the block alone. Its separator key, seam rank and
// the operations its parent buffers against it are the parent's to know,
// so they carry over from the row the parent produced. The dots change
// class in place rather than being replaced, which would detach them for
// a frame; a slotted dot stays even on a leaf, since an empty slot brings
// back the tree's chevron. A leaf stops being expandable.
async (self, item, hash) => {
  const state = self.__tree;
  let rows;
  try {
    rows = await self.query('tree/node', { hash });
  } catch {
    return;
  }
  if (rows.length === 0 || self.__tree !== state) return;
  const node = self.node(rows[0]);
  const previous = state.nodes.get(node.hash);
  if (previous) {
    if (previous.bound !== null) {
      node.bound = previous.bound;
      node.boundParts = previous.boundParts;
    }
    if (previous.rank !== null) node.rank = previous.rank;
    if (previous.pending !== null) node.pending = previous.pending;
  }
  state.nodes.set(node.hash, node);
  const old = item.querySelector(':scope > .row');
  if (old) item.replaceChild(self.row(node, null, null), old);

  const kit = self.kit();
  const locality = node.cached ? 'dot-local' : 'dot-remote';
  const slotted = ['expand-icon', 'collapse-icon']
    .map((slot) => item.querySelector(`:scope > [slot="${slot}"].dot`))
    .filter(Boolean);
  for (const dot of slotted) {
    dot.classList.remove('dot-local', 'dot-remote');
    dot.classList.add(locality);
  }
  item.querySelector(':scope > .dot-leaf')?.remove();
  if (slotted.length === 0) {
    const expand = kit.dot(node.cached);
    expand.setAttribute('slot', 'expand-icon');
    const collapse = kit.dot(node.cached);
    collapse.setAttribute('slot', 'collapse-icon');
    item.append(expand, collapse);
  }
  if (!(node.kind === 'index' && node.count > 0)) {
    item.removeAttribute('lazy');
    item.expanded = false;
  }
}
