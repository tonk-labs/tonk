(self, event) => {
  const button = event.target?.closest?.('[data-act]');
  if (!button || !self.contains(button)) return;
  const deck = self.getAttribute('deck');
  const edge = 'tonk:slide-deck/edge';
  const mine = self.ordered();
  const id = (card) => card?.dataset.slide ?? edge;
  const card = button.closest('.slide-card');
  const i = card ? mine.indexOf(card) : -1;
  const time = event.timeStamp;
  const act = button.dataset.act;
  if (act === 'start') self.emit('slide-insert', { deck, prev: edge, next: id(mine[0]), time });
  else if (i < 0) return;
  else if (act === 'add') self.emit('slide-insert', { deck, prev: id(card), next: id(mine[i + 1]), time });
  else if (act === 'up' && i > 0) self.emit('slide-move', { subject: id(card), prev: id(mine[i - 2]), next: id(mine[i - 1]), time });
  else if (act === 'down' && i < mine.length - 1) self.emit('slide-move', { subject: id(card), prev: id(mine[i + 1]), next: id(mine[i + 2]), time });
}
