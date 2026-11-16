// Build the shell once, then read the branch's root and watch it.
(self) => {
  if (!self.__tree) {
    const shadow = self.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = self.style();
    const shell = document.createElement('div');
    shell.className = 'shell';
    shadow.append(style, shell);
    self.__tree = {
      shadow,
      shell,
      // What is known of each node, by its `node:` entity.
      nodes: new Map(),
      // Decoded keys and separators, by the hex of their bytes.
      parts: new Map(),
      roots: [],
      selected: null,
      error: null,
      frame: 0,
      watch: null,
    };
    shell.addEventListener('click', (event) => self.act(event));
    shell.addEventListener('keydown', (event) => self.keys(event));
  }
  self.roots();
  self.watch();
  self.schedule();
}
