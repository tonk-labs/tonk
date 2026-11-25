(self) => {
  const s = self.__edit;
  if (!s || s.tag !== 'textarea' || !s.timer) return;
  clearTimeout(s.timer);
  s.timer = null;
  self.emitEdit(s.editor.value);
}
