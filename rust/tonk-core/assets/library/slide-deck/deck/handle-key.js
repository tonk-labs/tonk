(self, event) => {
  const s = self.__deck;
  if (!s?.deck.length || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || !self.keyboardOk(event)) return;
  const dir = { ArrowRight: 'right', ArrowLeft: 'left', ArrowDown: 'down', ArrowUp: 'up', PageDown: 'next', PageUp: 'prev', ' ': event.shiftKey ? 'prev' : 'next', n: 'next', p: 'prev' }[event.key];
  if (dir) { event.preventDefault(); self.step(dir); }
  else if (event.key === 'Home') { event.preventDefault(); self.show(0, 0); }
  else if (event.key === 'End') { event.preventDefault(); self.show(s.deck.length - 1, 0); }
  else if (event.key === 'f') self.goFullscreen();
}
