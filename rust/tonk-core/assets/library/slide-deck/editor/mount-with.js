(self, tag) => {
  const s = self.__edit;
  self.flushEdit();
  let editor;
  if (tag === 'prose-editor') {
    editor = document.createElement('prose-editor');
    editor.setAttribute('placeholder', 'One slide in markdown; a line Note: starts its speaker notes');
    editor.textContent = self.markdown();
    editor.addEventListener('change', (event) => {
      event.stopPropagation();
      self.emitEdit(editor.value);
    });
  } else if (tag === 'code-editor') {
    editor = document.createElement('code-editor');
    editor.setAttribute('language', 'markdown');
    editor.setAttribute('line-numbers', '');
    editor.setAttribute('value', self.markdown());
    editor.addEventListener('change', (event) => {
      event.stopPropagation();
      self.emitEdit(event.detail?.value ?? editor.value);
    });
  } else {
    editor = document.createElement('textarea');
    editor.spellcheck = true;
    editor.placeholder = 'One slide in markdown; a line Note: starts its speaker notes';
    editor.value = self.markdown();
    editor.addEventListener('input', () => {
      clearTimeout(s.timer);
      s.timer = setTimeout(() => self.emitEdit(editor.value), 600);
    });
    editor.addEventListener('blur', () => self.flushEdit());
    editor.addEventListener('change', (event) => event.stopPropagation());
  }
  s.editor = editor;
  s.tag = tag;
  s.mount.replaceChildren(editor);
  self.renderBar();
}
