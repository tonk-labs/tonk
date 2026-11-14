// Mount for the branch `with` names: the state the notebook keeps, the
// notice that stands in while the space lacks an editor component, and,
// once <prose-editor> is defined, the editor itself.
//
// The editor is created only after its tag is defined. A property written
// on an element before its definition upgrades it becomes an own property
// that hides the accessor, so a projection written into an undefined
// <prose-editor> would never reach the editor.
(self) => {
  const context = self.context();
  const error = self.querySelector(':scope > .notebook-error');
  if (!context) {
    if (!error) {
      const message = document.createElement('section');
      message.className = 'notebook-error error';
      message.textContent = 'no repository in context (nest under a with="branch@repo" element)';
      self.prepend(message);
    }
    return;
  }
  error?.remove();
  const state = (self.__notebook = {
    context,
    draft: self.hasAttribute('draft'),
    prose: null,
    notice: null,
    observer: null,
    timers: new Set(),
    // Fence wrappers already bound, by the cell id stamped on them.
    cells: new Map(),
    // The BASE: the blocks the editor's text was last brought in line with,
    // by a projection or a commit, in document order. An edit is the
    // editor's text diffed against these, never against rows that may
    // already carry another replica's edit.
    blocks: [],
    // Writes made that the rows may not show yet, each with the time
    // (`Date.now()`) after which the store wins again.
    pending: { edits: new Map(), keys: new Map(), removed: new Map(), inserts: [], insertsUntil: 0 },
    // The editor's text as of the base.
    projected: '',
    // Each block's row source as of the last projection, by entity.
    seen: new Map(),
    // The title last written, so a commit that leaves the heading alone
    // does not rename the notebook to itself.
    title: self.getAttribute('title') || null,
    // Whether the rows have been projected yet: until then an edit has
    // nothing truthful to diff against.
    projectedOnce: false,
    // The stylesheet that bands the block the caret is in (`mark`).
    band: null,
    selectionBound: false,
    // True while the notebook writes its own DOM.
    settling: false,
    detached: false,
    onSelection: () => self.mark(),
  });
  self.require();
  const attach = () => {
    if (self.__notebook === state) self.attach();
  };
  if (customElements.get('prose-editor')) {
    attach();
    return;
  }
  // Nothing renders the tag until it is defined, so ask for it.
  self.dispatchEvent(new CustomEvent('tonk-element-needed', {
    detail: { tag: 'prose-editor' }, bubbles: true, composed: true, cancelable: true,
  }));
  customElements.whenDefined('prose-editor').then(attach);
}
