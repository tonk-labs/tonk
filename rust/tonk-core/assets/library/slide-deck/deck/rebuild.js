(self) => {
  const s = self.__deck;
  if (!s) return;
  const source = self.collect();
  if (source === s.source) return;
  s.source = source;
  const deck = self.splitSlides(source);
  s.deck = deck;
  s.empty.hidden = deck.length > 0;
  s.stage.hidden = deck.length === 0;
  if (!deck.length) {
    s.empty.textContent = self.querySelector('.slide-part') ? 'This deck has no slides yet.' : 'Nothing to present.';
    self.update();
    return;
  }
  s.slides.innerHTML = deck
    .map((stack, h) => stack
      .map((slide, v) => `<section data-h="${h}" data-v="${v}">${self.toHtml(slide.body)}${slide.notes ? `<aside class="notes" hidden>${self.toHtml(slide.notes)}</aside>` : ''}</section>`)
      .join(''))
    .join('');
  if (!s.started) {
    s.started = true;
    const start = s.pending ?? self.getAttribute('start');
    s.pending = null;
    if (start) return self.goTo(start);
  }
  self.show(s.h, s.v);
}
