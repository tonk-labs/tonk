// Mark the top-level nodes of the block the caret is in.
//
// A block is what a commit writes, and it can span several nodes (a
// heading rides with what it introduces), so the whole run is marked, not
// the one node ProseMirror would.
//
// Drawn by a stylesheet of `:nth-child` rules beside the document, not a
// class on its nodes: ProseMirror reads an attribute written on a node it
// manages as an edit of the document and redraws the node, which drops the
// class the moment it is set.
//
// Only when the caret is in THIS notebook: notebooks nest (a cell's result
// can be a notebook), and every one listens to the same selection.
(self) => {
  const state = self.__notebook;
  const root = state?.prose?.shadowRoot;
  if (!root) return;
  const selection = root.getSelection?.() ?? window.getSelection();
  const anchor = selection?.anchorNode;
  if (!anchor || !root.contains(anchor)) return;
  let caret = -1;
  try {
    caret = state.prose.editor?.view.state.selection.$head.index(0) ?? -1;
  } catch {
    caret = -1;
  }
  if (!state.band) {
    state.band = document.createElement('style');
    root.append(state.band);
  }
  const span = caret < 0 ? null : self.blocks().spanAt(state.projected, caret);
  const rules = span ? self.band(span) : '';
  // Written only when it changes: a caret moving within its block, or a
  // selection event of the write's own making, leaves it alone.
  if (state.band.textContent !== rules) state.band.textContent = rules;
}
