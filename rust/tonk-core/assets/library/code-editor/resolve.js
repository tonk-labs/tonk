// Read the record for `word` and draw the editor with its pack. Called
// on mount and again whenever the subscription says the records changed:
// a language installed while the block is open, edited, or removed.
async (self, word) => {
  const state = self.__code;
  let source = null;
  try {
    const rows = await self.query(self.request(word));
    // Two records may claim one word; the lowest `this` wins, so every
    // editor on the page agrees on which.
    const sources = (Array.isArray(rows) ? rows : [])
      .map((row) => ({ id: String(row?.this ?? row?.fields?.this ?? ''), source: row?.fields?.source }))
      .filter((row) => typeof row.source === 'string' && row.source !== '')
      .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    source = sources[0]?.source ?? null;
  } catch (error) {
    // No host to ask: the editor stays as it is.
    console.warn(`<code-editor> could not read language "${word}":`, error);
    return;
  }
  if (state.wanted !== word || !state.editor) return;
  if (source === state.source) return;
  if (source === null) {
    // Removed: back to plain text.
    state.source = null;
    state.editor.setLanguage([]);
    return;
  }
  try {
    const extension = await self.constructor.__core.loadPack(source);
    if (state.wanted !== word || !state.editor) return;
    state.source = source;
    state.editor.setLanguage(extension);
  } catch (error) {
    console.warn(`<code-editor> language "${word}" did not load:`, error);
  }
}
