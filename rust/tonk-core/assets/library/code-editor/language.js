// Draw the editor in the language a block names by `word` (any of the
// language's words: `python`, `py`), or in none. The language is a
// record in the space, not something this element knows: whatever
// installed it, the palette or an author, it is found the same way. The
// pack in use stays until the next one is ready.
(self, word) => {
  const state = self.__code;
  const wanted = word ? String(word).trim().toLowerCase() || null : null;
  if (wanted === state.wanted && (state.watch || !wanted)) return;
  self.unwatch();
  state.wanted = wanted;
  if (!wanted) {
    state.source = null;
    state.editor?.setLanguage([]);
    return;
  }
  self.watch(wanted);
  self.resolve(wanted);
}
