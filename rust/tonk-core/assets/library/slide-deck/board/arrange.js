(self) => {
  const deck = self.getAttribute('deck');
  const list = self.querySelector(':scope > tonk-display') ?? self;
  const top = (node) => { let n = node; while (n.parentElement && n.parentElement !== list && n.parentElement !== self) n = n.parentElement; return n; };
  const show = (node, visible) => {
    const display = visible ? '' : 'none';
    if (node.style.display !== display) node.style.display = display;
  };
  for (const card of self.querySelectorAll('.slide-card')) {
    const visible = card.dataset.deck === deck;
    show(card, visible);
    show(top(card), visible);
  }
  const mine = self.ordered();
  mine.forEach((card, i) => {
    const node = top(card);
    if (node.style.order !== String(i)) node.style.order = String(i);
    const label = card.querySelector('.slide-number');
    const text = String(i + 1);
    if (label && label.textContent !== text) label.textContent = text;
  });
  for (const note of self.querySelectorAll(':scope > [data-when]')) {
    show(note, note.dataset.when === 'empty' ? mine.length === 0 : true);
  }
}
