// Build the shell once, then draw from the rows the console view keeps
// in `.ti-data`, again whenever they change.
(self) => {
  if (!self.__tree) {
    const shadow = self.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = self.sheet();
    const shell = document.createElement('div');
    shell.className = 'shell';
    // The rows stay in the light DOM, which a shadow root with no slot
    // never shows.
    shadow.append(style, shell);
    self.__tree = { shadow, shell, selected: null, pending: new Map(), frame: 0, observer: null, asked: new Set() };
    shell.addEventListener('click', (event) => self.act(event));
    shell.addEventListener('keydown', (event) => self.keys(event));
  }
  const state = self.__tree;
  state.observer?.disconnect();
  const data = self.querySelector(':scope > .ti-data');
  if (data) {
    state.observer = new MutationObserver(() => self.schedule());
    state.observer.observe(data, { childList: true, subtree: true, attributes: true });
  }
  self.schedule();
}
