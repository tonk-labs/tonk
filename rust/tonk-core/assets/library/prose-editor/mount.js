// Mount the ProseMirror view. The initial document is, in order: a
// property write that came before the mount, the light-DOM text, then
// the `content` or `value` attribute. Read through the envelope, so a
// later echo of that same version is recognised and dropped.
(self) => {
  const state = self.__prose;
  let raw = state.buffered;
  if (raw === null) {
    const light = self.textContent ?? '';
    raw = light !== '' ? light : (self.getAttribute('content') ?? self.getAttribute('value'));
  }
  let doc = '';
  if (raw !== null) {
    const { hlc, value } = self.decode(raw);
    doc = value;
    if (hlc !== null && hlc > state.known) state.known = self.observe(hlc);
  }
  // The core is evaluated once per tag and shared by every instance.
  const core = (self.constructor.__core ??= self.core());
  // Code blocks embed <code-editor> and stay plain monospace text until it
  // is defined. It is branch data too, resolved only once something asks
  // for the tag, and the editor never renders one before then, so it asks.
  // A space without the component leaves it unanswered; one that installs
  // it later defines it then, and the blocks redraw as editors.
  if (!customElements.get('code-editor')) {
    self.dispatchEvent(new CustomEvent('tonk-element-needed', {
      detail: { tag: 'code-editor' },
      bubbles: true,
      composed: true,
      cancelable: true,
    }));
  }
  const editor = core.createEditor(state.mount, {
    doc,
    readOnly: self.hasAttribute('readonly'),
    placeholder: self.getAttribute('placeholder') ?? '',
    code: 'code-editor',
    onChange: (value) => self.schedule(value),
    // The heading acts as a document switcher only where the host says
    // so, read once here: an editor that gained it later could navigate
    // away from a document being renamed.
    switcher: self.hasAttribute('switcher')
      ? {
          candidates: () => self.candidates,
          onOpen: (candidate) => self.emit('switch', candidate),
          onCreate: (title) => self.emit('create', { title, document: self.value }),
          onSuggest: (rows, active) => self.emit('suggest', { rows, active }),
        }
      : undefined,
  });
  state.buffered = null;
  state.editor = editor;
  self.emit('ready', { editor });

  if (!self.hasAttribute('readonly') && self.hasAttribute('auto-focus')) {
    // Deferred so sibling mounts settling in the same tick cannot take
    // focus back. The frame is claimed first: inside a nested guest an
    // unfocused frame leaves the caret invisible and the keys elsewhere.
    setTimeout(() => {
      if (state.editor !== editor) return;
      try { window.focus(); } catch {}
      if (self.getAttribute('caret') === 'end') editor.caretToEnd();
      editor.focus();
    }, 0);
  }
}
