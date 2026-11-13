// The index: a draft notebook whose heading is how you get to a notebook.
//
// The editor ranks the suggestions and handles the keys (its heading
// switcher); this is the page half. It hands the editor the notebooks the
// directory rendered as candidates, draws the panel of suggestions, opens
// the one picked, and creates the one named. The rows it reads are links,
// one per notebook, so with no script the index is still a list of them.
(self) => {
  if (self.__index) return;
  const index = (self.__index = { observer: null });
  if (!document.getElementById('notebook-index-style')) {
    const style = document.createElement('style');
    style.id = 'notebook-index-style';
    style.textContent = self.stylesheet();
    document.head.append(style);
  }
  if (!self.querySelector('.notebook-switcher__panel')) {
    const panel = document.createElement('div');
    panel.className = 'notebook-switcher__panel';
    panel.hidden = true;
    // Under the draft's editor, above the rows it matches against.
    const draft = self.querySelector('notebook-editor[draft]');
    const anchor = draft?.closest('notebook-save') ?? draft;
    if (anchor?.parentNode === self) anchor.after(panel);
    else self.append(panel);
  }
  self.addEventListener('suggest', (event) => self.panel(event.detail?.rows, event.detail?.active ?? 0));
  self.addEventListener('switch', (event) => {
    if (typeof event.detail?.href === 'string') self.navigate(event.detail.href);
  });
  self.addEventListener('create', (event) => {
    if (typeof event.detail?.title === 'string') self.create(event.detail.title, event.detail.document ?? '');
  });
  // The rows arrive from a render that may land after this: republish on
  // every change below.
  index.observer = new MutationObserver(() => self.publish());
  index.observer.observe(self, { childList: true, subtree: true });
  self.publish();
}
