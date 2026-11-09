// An incoming write: the store's text, or a `value`/`content` set by a
// script. A versioned write no newer than what this element issued or
// saw is its own echo (or stale) and is dropped, so a round trip never
// moves the caret. A bare write has no version and is always adopted.
(self, raw) => {
  const state = self.__prose;
  if (!state) return;
  const { hlc, value } = self.decode(raw);
  if (hlc !== null) {
    if (hlc <= state.known) return;
    state.known = self.observe(hlc);
  }
  if (!state.editor) {
    state.buffered = value;
    return;
  }
  // Patches only the span that differs, so the caret stays put.
  state.editor.setMarkdown(value);
}
