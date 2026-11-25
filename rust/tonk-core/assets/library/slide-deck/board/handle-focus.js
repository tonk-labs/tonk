(self, event) => {
  const card = event.target?.closest?.('.slide-card');
  const deck = self.getAttribute('deck');
  if (!card || card.dataset.deck !== deck) return;
  for (const preview of document.querySelectorAll('slide-deck')) {
    if (preview.getAttribute('deck') === deck) preview.showPart?.(card.dataset.slide);
  }
}
