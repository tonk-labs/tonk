// Commit as the author types, and when they leave the editor.
(self) => {
  const state = self.__notebook;
  const { prose } = state;
  // Every debounced `change` commits: the block diff writes only the block
  // that changed, and holding edits until the caret left a block lost them
  // on a reload and held back every other replica's edit behind them.
  //
  // A cell's <code-editor> fires `change` per keystroke, composed, so it
  // arrives here too; only the editor's own carries a `content` envelope,
  // and a cell's keystroke reaches the document, and so its next `change`,
  // anyway. Either stops here: the view may have bound commands to events
  // on this element.
  prose.addEventListener('change', (event) => {
    event.stopPropagation();
    self.mark();
    if (event.detail && typeof event.detail === 'object' && 'content' in event.detail) self.commit();
  });
  // The editor normalizes the text it was handed when it mounts; that is
  // the base, or the next projection reads the normalization as typing.
  // The editor's own `ready` only: every cell fires one too.
  prose.addEventListener('ready', (event) => {
    if (event.composedPath()[0] === prose && state.projectedOnce) self.settle();
  });
  // Leaving the editor commits at once.
  prose.addEventListener('focusout', () => self.commit());
  // The highlight follows the caret. A selection inside a shadow tree
  // fires `selectionchange` on its root, which exists once the editor
  // mounts (`fences` adds that listener); the document covers the time
  // before.
  document.addEventListener('selectionchange', state.onSelection);
}
