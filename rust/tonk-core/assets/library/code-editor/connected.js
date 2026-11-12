(self) => {
  let state = self.__code;
  if (!state) {
    // Open, and delegating focus, so focusing the host reaches the
    // editor without the page knowing it lives in a shadow root.
    const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open', delegatesFocus: true });
    const style = document.createElement('style');
    shadow.replaceChildren(style);
    state = self.__code = {
      shadow, style, editor: null, teardown: false,
      // The language asked for (lower-cased), the pack it is drawn
      // with, and the subscription that hears it installed or changed.
      wanted: null, source: null, watch: null,
      // The language server: the source announced, the session that
      // took it (null when an ancestor supplied the client), the client
      // attached and what it opened, and the latest frame's errors.
      announced: null, server: null, client: null, attachment: null, errors: 0,
    };
    // Delegated focus lands on CodeMirror's scroller, which is focusable,
    // not on its contenteditable; send it on, however the host got it.
    self.addEventListener('focus', () => self.__code?.editor?.view.focus());
  }
  // A re-attach inside the teardown deferral keeps the live editor, with
  // its undo history and selection.
  state.teardown = false;
  if (!state.editor) self.mount();
  else if (state.wanted && !state.watch) self.watch(state.wanted);
}
