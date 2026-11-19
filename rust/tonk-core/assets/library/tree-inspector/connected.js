// Build the shell once, read what the page shows, and follow the
// session's explorations and the branch's root for changes.
(self) => {
  if (!self.__tree) {
    const shadow = self.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = self.sheet();
    const shell = document.createElement('div');
    shell.className = 'shell';
    shadow.append(style, shell);
    self.__tree = { shadow, shell, style, selected: null, pending: new Map(), frame: 0, asked: new Set(), model: null, watches: null };
    shell.addEventListener('click', (event) => self.act(event));
    shell.addEventListener('keydown', (event) => self.keys(event));
  }
  self.watch();
  self.refresh();
  self.schedule();
}
