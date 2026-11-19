(self, name, before, after) => {
  const state = self.__prose;
  // The runtime replays attributes before `connected`, which reads them
  // when it mounts.
  if (!state) return;
  if (name === 'content') self.adopt(after ?? '');
  // `value` is the bare channel; reflected only when it differs, so it
  // does not fight the author's keystrokes.
  else if (name === 'value' && (after ?? '') !== self.value) self.adopt(after ?? '');
  else if (name === 'readonly') state.editor?.setReadOnly(after !== null);
  else if (name === 'placeholder') state.editor?.setPlaceholder(after ?? '');
}
