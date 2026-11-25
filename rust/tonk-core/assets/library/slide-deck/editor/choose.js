(self) => {
  const s = self.__edit;
  const tags = self.preference();
  const ready = tags.find((tag) => customElements.get(tag));
  self.mountWith(ready ?? 'textarea');
  if (s.waiting) return;
  s.waiting = true;
  for (const tag of tags) {
    if (customElements.get(tag)) continue;
    self.dispatchEvent(new CustomEvent('tonk-element-needed', { detail: { tag }, bubbles: true, composed: true, cancelable: true }));
    customElements.whenDefined(tag).then(() => {
      const now = self.__edit;
      if (!now) return;
      self.renderBar();
      if (now.tag === 'textarea' && !self.matches(':focus-within')) self.mountWith(tag);
    });
  }
}
