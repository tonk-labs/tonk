(self) => {
  if (self.__reader) return;
  const branch = self.getAttribute('with');
  if (!window.tonk || !branch) return;
  if (!self.__style) {
    self.__style = document.createElement('style');
    self.__style.textContent = `
      task-console { display: block; box-sizing: border-box; min-height: 100dvh;
        padding: 88px 16px 48px; font: 15px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif; }
      task-console h1 { margin: 0 0 16px; font-size: 22px; }
      task-console .empty { opacity: .7; }
      task-console ol { list-style: none; margin: 0; padding: 0; max-width: 760px; }
      task-console li { border-top: 1px solid color-mix(in srgb, currentColor 18%, transparent); }
      task-console button.row { all: unset; box-sizing: border-box; width: 100%; cursor: pointer;
        display: grid; grid-template-columns: 7.5em minmax(0, 1fr) auto; gap: 12px; padding: 10px 4px; }
      task-console button.row:focus-visible { outline: 2px solid currentColor; outline-offset: -2px; }
      task-console .case { font-weight: 600; }
      task-console [data-case="failed"] .case, task-console [data-case="aborted"] .case,
      task-console [data-case="unhandled"] .case { color: #b42318; }
      task-console .what { min-width: 0; overflow-wrap: anywhere; }
      task-console .when { opacity: .7; white-space: nowrap; }
      task-console .detail { padding: 0 4px 12px 8.5em; }
      @media (prefers-color-scheme: dark) {
        task-console [data-case="failed"] .case, task-console [data-case="aborted"] .case,
        task-console [data-case="unhandled"] .case { color: #ff8a80; }
      }`;
    self.append(self.__style);
  }
  self.__rows = new Map();
  self.__open = self.__open || new Set();
  const one = (the, as) => ({ the, as, cardinality: 'one' });
  self.__one = one;
  self.__reader = window.tonk.subscribe({
    predicate: { with: {
      case: one('xyz.tonk.invocation/case', 'Entity'),
      started: one('xyz.tonk.invocation/started', 'Float'),
    } },
    terms: { this: { '?': { name: 'this' } }, case: { '?': { name: 'case' } },
      started: { '?': { name: 'started' } } },
  }, { with: branch }).getReader();
  const reader = self.__reader;
  (async () => {
    try {
      while (self.__reader === reader) {
        const { value, done } = await reader.read();
        if (done) return;
        self.__rows = new Map((value || []).map((row) => [row.this, row.fields || {}]));
        await self.detail();
        self.render();
      }
    } catch (error) {
      console.warn('<task-console> stopped', error);
    }
  })();
  self.render();
}
