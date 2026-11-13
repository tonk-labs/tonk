// Bind every dialog fence in the document not bound yet. Idempotent: the
// id stamped on a fence's wrapper is what makes a re-scan cheap.
//
// The editor draws each code block as a `.md-code-block` wrapping a
// <code-editor>, a node view that never rebuilds the wrapper and ignores
// mutations inside it, so a result appended there survives edits and is
// chrome around the fence, never document content.
//
// A fence is a cell when it is tagged `dialog` or `dialog-yaml`, or not
// tagged at all: in a notebook the common case is a query.
(self) => {
  const state = self.__notebook;
  const root = state?.prose?.shadowRoot;
  if (!root) return;
  if (!root.__notebookStyled) {
    const style = document.createElement('style');
    style.textContent = self.stylesheet();
    root.append(style);
    root.__notebookStyled = true;
  }
  if (!state.selectionBound) {
    root.addEventListener('selectionchange', state.onSelection);
    state.selectionBound = true;
  }
  for (const wrapper of root.querySelectorAll('.md-code-block')) {
    const editor = wrapper.querySelector('code-editor');
    if (!editor) continue;
    const language = editor.getAttribute('language') ?? '';
    if (language && language !== 'dialog' && language !== 'dialog-yaml') continue;
    if (language !== 'dialog-yaml') editor.setAttribute('language', 'dialog-yaml');
    // A counter, not the fence's index: a fence appearing where another
    // was bound is another cell. Shared across notebooks, because the id
    // names the cell's language-server buffer, and two editors on one
    // buffer would share a document.
    let id = wrapper.dataset.notebookCell;
    if (id === undefined) {
      id = String((globalThis.__notebookCells = (globalThis.__notebookCells ?? 0) + 1) - 1);
      wrapper.dataset.notebookCell = id;
    }
    if (state.cells.has(id)) continue;
    state.cells.set(id, self.cell(wrapper, editor, id));
  }
}
