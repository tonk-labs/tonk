(self, dir) => {
  const s = self.__deck;
  const deck = s?.deck;
  if (!deck?.length) return;
  if (dir === 'right') self.show(s.h + 1, 0);
  else if (dir === 'left') self.show(s.h - 1, 0);
  else if (dir === 'down') self.show(s.h, s.v + 1);
  else if (dir === 'up') self.show(s.h, s.v - 1);
  else if (dir === 'next') {
    if (s.v < deck[s.h].length - 1) self.show(s.h, s.v + 1);
    else self.show(s.h + 1, 0);
  } else if (dir === 'prev') {
    if (s.v > 0) self.show(s.h, s.v - 1);
    else if (s.h > 0) self.show(s.h - 1, deck[s.h - 1].length - 1);
  }
}
