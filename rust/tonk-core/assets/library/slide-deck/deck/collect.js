(self) => {
  const deck = self.getAttribute('deck');
  const parts = new Map();
  for (const row of self.querySelectorAll('.slide-part')) {
    if (row.dataset.deck !== deck) continue;
    const id = row.dataset.part || '';
    const at = row.dataset.at ?? '';
    const known = parts.get(id);
    if (known && !(at < known.at)) continue;
    const text = self.stripEnvelope(row.textContent ?? '').replace(/^\n+/, '');
    const below = row.dataset.below;
    const separator = below === undefined ? '' : (below === 'true' || below === '1' ? '--\n\n' : '---\n\n');
    parts.set(id, { id, at, text: separator + text });
  }
  const order = [...parts.values()].sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
  if (self.__deck) self.__deck.order = order;
  return order.map((part) => part.text.replace(/^\n+|\s+$/g, '')).join('\n\n');
}
