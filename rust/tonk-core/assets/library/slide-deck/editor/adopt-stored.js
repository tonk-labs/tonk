(self) => {
  const s = self.__edit;
  if (!s?.editor) return;
  const stored = self.stored();
  const markdown = self.markdown();
  if (stored === s.last || markdown === s.last) return;
  if (self.current() === markdown) return;
  if (s.tag === 'prose-editor') {
    s.editor.content = markdown;
  } else if (s.tag === 'code-editor') {
    s.editor.setAttribute('value', markdown);
  } else if (document.activeElement !== self) {
    s.editor.value = markdown;
  }
}
