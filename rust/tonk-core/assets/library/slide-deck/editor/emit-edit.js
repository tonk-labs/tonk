(self, source) => {
  const s = self.__edit;
  if (typeof source !== 'string' || source === s.last) return;
  s.last = source;
  self.dispatchEvent(new CustomEvent('slide-edit', { bubbles: true, detail: { source } }));
}
