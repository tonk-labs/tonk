(self) => {
  let state = self.__table;
  if (!state) {
    // The core is evaluated once per tag and shared by every instance.
    const core = (self.constructor.__core ??= self.core());
    // The light DOM is the content channel (text, or the claim rows), so
    // the grid mounts in a shadow root the store's writes never reach.
    const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open', delegatesFocus: true });
    const style = document.createElement('style');
    style.textContent = core.SHADOW_STYLESHEET;
    const mount = document.createElement('div');
    mount.className = 'mount';
    // The element's own `<style>` children, adopted last so they win.
    const user = document.createElement('style');
    shadow.replaceChildren(style, mount, user);
    state = self.__table = {
      core, mount, user, grid: null, token: 0, pending: null, light: null,
      observer: null, rows: null, timer: null, dirty: false,
      clock: new core.Clock(), known: 0n, teardown: false,
    };
  }
  // A re-attach inside the teardown deferral keeps the live grid, with
  // its undo history and selection.
  state.teardown = false;
  if (!state.observer) {
    state.observer = new MutationObserver(() => {
      self.restyle();
      if (self.hasAttribute('subject')) self.rows();
      else self.relight();
    });
    state.observer.observe(self, {
      childList: true, characterData: true, subtree: true,
      attributes: true, attributeFilter: state.core.ROW_ATTRS,
    });
  }
  self.restyle();
  if (!state.grid) self.mount(++state.token);
}
