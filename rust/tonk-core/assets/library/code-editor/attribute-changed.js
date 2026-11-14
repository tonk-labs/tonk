(self, name, before, after) => {
  const state = self.__code;
  const editor = state?.editor;
  // The runtime replays attributes before `connected`, which reads them
  // when it mounts.
  if (!editor) return;
  // Reflected only when it differs from the buffer, so it does not
  // fight the author's keystrokes.
  if (name === 'value') { if ((after ?? '') !== self.value) self.value = after ?? ''; }
  else if (name === 'language') {
    self.language(after);
    // The server opened the document under the old language id.
    if (state.announced) { self.withdraw(); self.announce(); }
  }
  // A new document, or a new server: close the old, open the new.
  else if (name === 'source' || name === 'language-server') { self.withdraw(); self.announce(); }
  else if (name === 'readonly') editor.setReadOnly(after !== null);
  else if (name === 'placeholder') editor.setPlaceholder(after ?? '');
  else if (name === 'line-numbers') editor.setLineNumbers(after !== null);
  else if (name === 'active-line') editor.setActiveLine(after !== null);
}
