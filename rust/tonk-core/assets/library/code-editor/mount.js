// Mount the CodeMirror view from the attributes, then ask for the
// language. The editor is usable at once: a language arrives when its
// pack does, and a block in one the space lacks stays plain text.
(self) => {
  const state = self.__code;
  // The core is evaluated once per tag and shared by every instance, so
  // every editor and every pack share one CodeMirror.
  const core = (self.constructor.__core ??= self.core());
  state.style.textContent = core.SHADOW_STYLESHEET;
  const editor = core.createEditor(state.shadow, {
    doc: self.getAttribute('value') ?? '',
    readOnly: self.hasAttribute('readonly'),
    placeholder: self.getAttribute('placeholder') ?? '',
    lineNumbers: self.hasAttribute('line-numbers'),
    activeLine: self.hasAttribute('active-line'),
    // User edits only: a `value` write does not come back as a change.
    onChange: (update) => self.emit('change', { value: update.state.doc.toString(), doc: update.state.doc }),
    // Shift+Enter or Mod+Enter, consumed by the editor.
    onRun: () => {
      const doc = state.editor?.view.state.doc;
      if (doc) self.emit('run', { value: doc.toString(), doc });
    },
    // A transaction carrying diagnostics is a fresh frame from the
    // language server, for the version it was sent: the verdict on the
    // text as typed. Remapped ranges on an edit are not one.
    onUpdate: (update) => {
      const lint = core.shared['@codemirror/lint'];
      const fresh = update.transactions.some((tr) => tr.effects.some((effect) => effect.is(lint.setDiagnosticsEffect)));
      if (!fresh) return;
      let count = 0;
      let errors = 0;
      lint.forEachDiagnostic(update.state, (diagnostic) => {
        count += 1;
        if (diagnostic.severity === 'error') errors += 1;
      });
      state.errors = errors;
      self.emit('diagnostics', { value: update.state.doc.toString(), doc: update.state.doc, count, errorCount: errors });
    },
  });
  state.editor = editor;
  self.language(self.getAttribute('language'));
  // Before `ready`: attaching reconfigures the editor, which can take
  // focus from whoever handles `ready`.
  self.announce();
  self.emit('ready', { view: editor.view });

  if (!self.hasAttribute('readonly') && self.hasAttribute('auto-focus')) {
    // Deferred so sibling mounts settling in the same tick cannot take
    // focus back.
    setTimeout(() => { if (state.editor === editor) editor.view.focus(); }, 0);
  }
}
