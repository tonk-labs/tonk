(self, n) => {
  const s = self.__deck;
  if (!s) return;
  let k = Math.max(1, parseInt(n, 10) || 1);
  if (!s.started) { s.pending = k; return; }
  for (let h = 0; h < s.deck.length; h++) {
    if (k <= s.deck[h].length) return self.show(h, k - 1);
    k -= s.deck[h].length;
  }
  if (s.deck.length) self.show(s.deck.length - 1, s.deck.at(-1).length - 1);
}
