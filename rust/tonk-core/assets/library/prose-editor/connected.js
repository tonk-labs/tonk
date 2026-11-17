(self) => {
  let state = self.__prose;
  if (!state) {
    // The light DOM is the content channel, so the editor mounts in a
    // shadow root the store's text writes never reach.
    const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open', delegatesFocus: true });
    const style = document.createElement('style');
    const mount = document.createElement('div');
    mount.className = 'mount';
    shadow.replaceChildren(style, mount);
    state = self.__prose = {
      style, mount, editor: null, observer: null,
      known: 0n, clock: 0n, timer: null, pending: null, buffered: null, teardown: false,
    };
  }
  state.style.textContent = self.stylesheet();
  // A re-attach inside the teardown deferral keeps the live editor, with
  // its undo history and selection.
  state.teardown = false;
  if (!state.observer) {
    state.observer = new MutationObserver(() => self.adopt(self.textContent ?? ''));
    state.observer.observe(self, { childList: true, characterData: true, subtree: true });
  }
  if (!state.editor) self.mount();
}
