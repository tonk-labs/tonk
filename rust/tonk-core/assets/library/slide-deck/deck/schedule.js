(self) => {
  const s = self.__deck;
  if (!s) return;
  clearTimeout(s.timer);
  s.timer = setTimeout(() => {
    try { self.rebuild(); } catch (error) { console.error('slide-deck', error); }
  }, 120);
}
