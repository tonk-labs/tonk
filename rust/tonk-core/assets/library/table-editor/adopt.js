// An incoming write: the store's text, or a `value`/`content` set by a
// script. A versioned write no newer than what this element issued or
// saw is its own echo (or stale) and is dropped, so a round trip never
// reloads the grid under the user. Bare CSV has no version and is always
// adopted. With a `subject` the rows are the document, so there is no
// content channel to write.
(self, raw) => {
  const state = self.__table;
  if (!state || self.hasAttribute('subject')) return;
  const parsed = state.core.parseContent(raw);
  if (parsed.hlc !== null) {
    if (parsed.hlc <= state.known) return;
    state.known = state.clock.receive(parsed.hlc);
  }
  if (!state.grid) {
    state.pending = raw;
    return;
  }
  state.grid.load(state.core.toSource(parsed));
}
