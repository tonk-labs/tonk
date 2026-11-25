(self) => {
  let s = self.__edit;
  if (!s) {
    const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open', delegatesFocus: true });
    const style = document.createElement('style');
    const bar = document.createElement('div');
    bar.className = 'bar';
    const mount = document.createElement('div');
    mount.className = 'mount';
    shadow.replaceChildren(style, bar, mount);
    s = self.__edit = { style, bar, mount, editor: null, tag: null, timer: null, observer: null, last: null, waiting: false };
  }
  s.style.textContent = self.editorStyle();
  if (!s.observer) {
    s.observer = new MutationObserver(() => self.adoptStored());
    s.observer.observe(self, { childList: true, characterData: true, subtree: true });
  }
  if (!s.editor) self.choose();
}
