(self) => {
  const s = self.__deck;
  const deck = s.deck;
  const total = deck.reduce((n, stack) => n + stack.length, 0);
  let n = s.v + 1;
  for (let i = 0; i < s.h && i < deck.length; i++) n += deck[i].length;
  s.counter.textContent = total ? `${n} / ${total}` : '';
  s.progress.style.width = total ? `${(n / total) * 100}%` : '0';
  const can = { left: s.h > 0, right: s.h < deck.length - 1, up: s.v > 0, down: Boolean(deck[s.h]) && s.v < deck[s.h].length - 1 };
  for (const button of s.controls.children) button.disabled = !can[button.dataset.dir];
}
