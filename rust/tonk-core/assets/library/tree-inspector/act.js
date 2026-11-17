// A click: open or close a node (a command, answered when the session
// holds it), select a row, show more entries, or start an exploration.
(self, event) => {
  const state = self.__tree;
  const explore = event.target.closest?.('[data-explore]');
  if (explore) {
    // The link navigates; the exploration is started beside it.
    state.asked.add(explore.dataset.explore);
    self.send('tree-explore', { root: explore.dataset.explore });
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
