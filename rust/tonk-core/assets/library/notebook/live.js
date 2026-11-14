// The editor's live markdown, or null before there is an editor.
(self) => {
  const value = self.__notebook?.prose?.value;
  return typeof value === 'string' ? value : null;
}
