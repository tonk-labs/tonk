(self) => {
  const s = self.__edit;
  if (!s?.editor) return null;
  if (s.tag === 'textarea') return s.editor.value;
  try { return s.editor.value; } catch (_) { return null; }
}
