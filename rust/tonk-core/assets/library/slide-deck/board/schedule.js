(self) => {
  const s = self.__board;
  if (!s || s.frame) return;
  s.frame = requestAnimationFrame(() => { s.frame = 0; self.arrange(); });
}
