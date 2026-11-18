// A click: open or close a node (a command, answered when the session
// holds it), select a row, show more entries, or start an exploration.
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
  const read = event.target.closest?.('[data-value]');
  if (read) {
    // A spilled value is opened in the exploration like a node.
    state.pending.set(read.dataset.value, 'open');
    self.send('tree-open', { exploration: self.getAttribute('selected') || '', node: read.dataset.value });
    self.schedule();
    return;
  }
  const row = event.target.closest?.('.row');
  if (!row) return;
  if (row.dataset.more) {
    state.shown ??= new Map();
    state.shown.set(row.dataset.more, (state.shown.get(row.dataset.more) ?? 100) + 100);
    self.schedule();
    return;
  }
  const exploration = self.getAttribute('selected') || '';
  if (row.dataset.node) {
    const node = row.dataset.node;
    const open = state.model?.explorations.get(exploration)?.open.has(node);
    const twisted = !!event.target.closest('.dot');
    if (open && (twisted || state.selected === node)) {
      state.pending.set(node, 'close');
      self.send('tree-close', { exploration, node });
    } else if (!open) {
      state.pending.set(node, 'open');
      self.send('tree-open', { exploration, node });
    }
    state.selected = node;
  } else if (row.dataset.entry) {
    state.selected = row.dataset.entry;
  } else {
    return;
  }
  self.schedule();
}
