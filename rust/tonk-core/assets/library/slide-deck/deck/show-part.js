(self, id) => {
  const s = self.__deck;
  if (!s?.order) return;
  let n = 1;
  for (const part of s.order) {
    if (part.id === id) return self.goTo(n);
    n += self.splitSlides(part.text).flat().length;
  }
}
