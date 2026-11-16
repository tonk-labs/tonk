// A click in the outline or the pane: open or close a node, select a
// row, show more entries, read a value.
(self, event) => {
  const state = self.__tree;
  const button = event.target.closest?.('[data-value]');
  if (button) {
    for (const node of state.nodes.values()) {
      const entry = (node.entries ?? []).find((candidate) => String(candidate.entry ?? `${node.node}/${candidate.at}`) === button.dataset.value);
      if (entry) { self.value(entry); return; }
    }
    return;
  }
  const row = event.target.closest?.('.row');
  if (!row) return;
  if (row.dataset.more) {
    const known = state.nodes.get(row.dataset.more);
    const from = known.shown;
    known.shown += 100;
    for (const entry of known.entries.slice(from, known.shown)) self.label('dialog/key-part', entry.key);
    self.schedule();
    return;
  }
  if (row.dataset.node) {
    const node = row.dataset.node;
    const known = state.nodes.get(node);
    // The twist opens and closes; the rest of the row selects, and
    // opens a node not yet read.
    const twisted = !!event.target.closest('.twist');
    if (known?.open && (twisted || state.selected === node)) known.open = false;
    else self.open(node);
    state.selected = node;
  } else if (row.dataset.entry) {
    state.selected = row.dataset.entry;
  } else {
    return;
  }
  self.schedule();
}
