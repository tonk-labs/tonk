(self) => {
  let s = self.__deck;
  if (s && !s.stage) { self.shadowRoot?.replaceChildren(); s = self.__deck = null; }
  if (!s) {
    const shadow = self.shadowRoot ?? self.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    const stage = document.createElement('div');
    stage.className = 'stage';
    const slides = document.createElement('div');
    slides.className = 'slides';
    stage.append(slides);
    const empty = document.createElement('div');
    empty.className = 'empty';
    empty.textContent = 'Loading slides…';
    const progress = document.createElement('div');
    progress.className = 'progress';
    const bar = document.createElement('div');
    bar.className = 'bar';
    const counter = document.createElement('span');
    counter.className = 'counter';
    const full = document.createElement('button');
    full.type = 'button';
    full.textContent = 'Fullscreen';
    full.addEventListener('click', () => self.goFullscreen());
    bar.append(counter, full);
    const controls = document.createElement('div');
    controls.className = 'controls';
    for (const [dir, label] of [['up', '↑'], ['left', '←'], ['right', '→'], ['down', '↓']]) {
      const button = document.createElement('button');
      button.type = 'button';
      button.dataset.dir = dir;
      button.textContent = label;
      button.setAttribute('aria-label', dir);
      button.addEventListener('click', () => self.step(dir));
      controls.append(button);
    }
    shadow.replaceChildren(style, stage, empty, progress, controls, bar);
    s = self.__deck = {
      style, stage, slides, empty, progress, counter, controls,
      deck: [], h: 0, v: 0, source: null, timer: null, observer: null,
      resize: null, onkey: null, order: null, started: false, pending: null,
    };
  }
  s.style.textContent = self.baseStyle();
  if (!self.hasAttribute('tabindex')) self.setAttribute('tabindex', '0');
  if (!s.observer) {
    s.observer = new MutationObserver(() => self.schedule());
    s.observer.observe(self, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['data-at', 'data-deck', 'data-part', 'data-below', 'deck'] });
  }
  if (!s.resize) {
    s.resize = new ResizeObserver(() => self.fit());
    s.resize.observe(self);
  }
  if (!s.onkey) {
    s.onkey = (event) => self.handleKey(event);
    document.addEventListener('keydown', s.onkey);
  }
  self.fit();
  self.schedule();
}
