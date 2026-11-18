// A click: follow a link, read a spilled value, unfold an entry, show
// more entries, or select a node. Selecting a node opens it (a command,
// answered when the session holds it), so its entries can be read; the
// dot, or selecting an open index node again, closes it.
(self, event) => {
  const state = self.__tree;
  const link = event.target.closest?.('a[href]');
  if (link) {
    // The page follows a link by the anchor a click came from, which a
    // shadow root hides; follow it through one in the light DOM.
    event.preventDefault();
    if (link.dataset.explore) {
      state.asked.add(link.dataset.explore);
      self.send('tree-explore', { root: link.dataset.explore });
    }
    const relay = document.createElement('a');
    relay.href = link.getAttribute('href');
    relay.hidden = true;
    self.append(relay);
    relay.click();
    relay.remove();
    return;
  }
  const exploration = self.getAttribute('selected') || '';
  const read = event.target.closest?.('[data-value]');
  if (read) {
    // A spilled value is opened in the exploration like a node.
    state.pending.set(read.dataset.value, 'open');
    self.send('tree-open', { exploration, node: read.dataset.value });
    self.schedule();
    return;
  }
  const more = event.target.closest?.('[data-more]');
  if (more) {
    state.shown ??= new Map();
    state.shown.set(more.dataset.more, (state.shown.get(more.dataset.more) ?? 200) + 200);
    self.schedule();
    return;
  }
  const unfold = event.target.closest?.('[data-unfold]');
  if (unfold) {
    state.unfolded ??= new Set();
    const entry = unfold.dataset.unfold;
    if (state.unfolded.has(entry)) state.unfolded.delete(entry);
    else state.unfolded.add(entry);
    self.schedule();
    return;
  }
  const row = event.target.closest?.('.row');
  if (!row?.dataset.node) return;
  const node = row.dataset.node;
  const open = state.model?.explorations.get(exploration)?.open.has(node);
  const index = state.model?.nodes.get(node)?.kind === 'index';
  const twisted = !!event.target.closest('.dot');
  if (open && (twisted || (index && state.selected === node))) {
    state.pending.set(node, 'close');
    self.send('tree-close', { exploration, node });
  } else if (!open) {
    state.pending.set(node, 'open');
    self.send('tree-open', { exploration, node });
  }
  state.selected = node;
  self.schedule();
}
