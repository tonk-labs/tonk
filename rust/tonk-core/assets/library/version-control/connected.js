(self) => {
  if (!self.__shell) {
    self.__shell = document.createElement('div');
    self.__shell.className = 'vc-shell';
    self.__drafts = new Map();
    self.__keys = (event) => {
      const field = event.target.closest?.('textarea, input, [contenteditable]');
      if (field || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === 'j' || event.key === 'ArrowDown') {
        event.preventDefault();
        self.navigate(1);
      } else if (event.key === 'k' || event.key === 'ArrowUp') {
        event.preventDefault();
        self.navigate(-1);
      }
    };
    self.addEventListener('keydown', self.__keys);
  }
  if (self.__shell.parentNode !== self) self.appendChild(self.__shell);
  const data = self.querySelector(':scope > .vc-data');
  self.__observer?.disconnect();
  if (data) {
    self.__observer = new MutationObserver(() => self.schedule());
    self.__observer.observe(data, { childList: true, subtree: true, attributes: true });
  }
  self.schedule();
}
