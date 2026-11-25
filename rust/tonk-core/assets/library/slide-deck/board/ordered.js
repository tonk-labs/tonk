(self) => {
  const deck = self.getAttribute('deck');
  const at = (card) => card.dataset.at ?? '';
  return [...self.querySelectorAll('.slide-card')]
    .filter((card) => card.dataset.deck === deck)
    .sort((a, b) => (at(a) < at(b) ? -1 : at(a) > at(b) ? 1 : 0));
}
