(self, h, v) => {
  const s = self.__deck;
  if (!s) return;
  const deck = s.deck;
  if (!deck.length) return self.update();
  s.h = Math.max(0, Math.min(h, deck.length - 1));
  s.v = Math.max(0, Math.min(v, deck[s.h].length - 1));
  for (const section of s.slides.children) {
    const sh = Number(section.dataset.h);
    const sv = Number(section.dataset.v);
    const state = sh === s.h && sv === s.v ? 'present' : sh < s.h || (sh === s.h && sv < s.v) ? 'past' : 'future';
    if (section.dataset.state !== state) section.dataset.state = state;
    const axis = sh === s.h ? 'v' : 'h';
    if (section.dataset.axis !== axis) section.dataset.axis = axis;
  }
  self.update();
}
