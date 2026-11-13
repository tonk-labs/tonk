// Draw the suggestions: every row the same shape, the name (its matched
// characters marked, unless it is the create row, which is what was typed)
// and a small verb saying what Enter does with it.
(self, rows, active) => {
  const panel = self.querySelector('.notebook-switcher__panel');
  if (!panel) return;
  panel.replaceChildren();
  if (!Array.isArray(rows) || rows.length === 0) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  rows.forEach((row, index) => {
    const title = String(row?.title ?? '');
    const create = !!row?.create;
    const item = document.createElement('div');
    item.className = 'notebook-switcher__row'
      + (index === active ? ' notebook-switcher__row--active' : '')
      + (create ? ' notebook-switcher__row--create' : '');
    item.dataset.index = String(index);
    const name = document.createElement('span');
    name.className = 'notebook-switcher__name';
    if (create) {
      name.textContent = title;
    } else {
      // Runs, not characters: a <mark> per stretch, so the text stays
      // selectable as words.
      const marked = new Set(Array.isArray(row?.spans) ? row.spans.map(Number) : []);
      const chars = Array.from(title);
      for (let at = 0; at < chars.length;) {
        const hit = marked.has(at);
        let end = at;
        while (end < chars.length && marked.has(end) === hit) end += 1;
        const part = document.createElement(hit ? 'mark' : 'span');
        part.textContent = chars.slice(at, end).join('');
        name.append(part);
        at = end;
      }
    }
    const verb = document.createElement('span');
    verb.className = 'notebook-switcher__verb';
    verb.textContent = create ? 'create' : 'open';
    item.append(name, verb);
    panel.append(item);
  });
}
