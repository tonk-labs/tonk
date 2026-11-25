(self) => {
  self.__board ??= { observer: null, frame: 0, onclick: null, onfocus: null };
  const s = self.__board;
  if (!s.observer) {
    s.observer = new MutationObserver(() => self.schedule());
    s.observer.observe(self, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-deck', 'data-at', 'deck'] });
  }
  if (!s.onclick) {
    s.onclick = (event) => self.handleClick(event);
    self.addEventListener('click', s.onclick);
  }
  if (!s.onfocus) {
    s.onfocus = (event) => self.handleFocus(event);
    self.addEventListener('focusin', s.onfocus);
  }
  self.schedule();
}
