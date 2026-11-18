(self, name, before, after) => {
  const editor = self.__code?.editor;
  // The runtime replays attributes before `connected`, which reads them
  // when it mounts.
  if (!editor) return;
  // Reflected only when it differs from the buffer, so it does not
  // fight the author's keystrokes.
  if (name === 'value') { if ((after ?? '') !== self.value) self.value = after ?? ''; }
  else if (name === 'language') self.language(after);
  else if (name === 'readonly') editor.setReadOnly(after !== null);
  else if (name === 'placeholder') editor.setPlaceholder(after ?? '');
  else if (name === 'line-numbers') editor.setLineNumbers(after !== null);
  else if (name === 'active-line') editor.setActiveLine(after !== null);
}
