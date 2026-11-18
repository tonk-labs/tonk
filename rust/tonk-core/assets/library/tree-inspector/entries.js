// A node's entries as a table: Index · Entity · Attribute · Value, the
// columns read off each key's parts. A segment holds more than facts
// (history and coverage records, blob records), so the ordering has a
// cell of its own and the other three say what the row holds. A row
// unfolds into everything the entry carries. An index node's rows are the
// operations it buffers, with the span each rides.
(self, model, node, entries, buffered) => {
  const state = self.__tree;
  state.unfolded ??= new Set();
  const element = (tag, className, text) => {
    const item = document.createElement(tag);
    if (className) item.className = className;
    if (text !== undefined) item.textContent = text;
    return item;
  };
  const trunc = (text, n) => {
    const chars = [...String(text)];
    return chars.length > n ? `${chars.slice(0, n).join('')}…` : String(text);
  };
  const kv = (name, value) => {
    const line = element('div', 'kv');
    line.append(element('span', 'k', name), element('span', 'v', String(value)));
    return line;
  };
  const of = (parts, kind) => parts?.find((piece) => piece.kind === kind) ?? null;
  const spans = model.spans.get(node) ?? [];

  const table = element('table', 'entries');
  const head = element('tr');
  for (const name of buffered ? ['Index', 'Entity', 'Attribute', 'Value', 'Span'] : ['Index', 'Entity', 'Attribute', 'Value']) {
    head.append(element('th', '', name));
  }
  const thead = element('thead');
  thead.append(head);
  const tbody = element('tbody');
  table.append(thead, tbody);

  const shown = state.shown?.get(node) ?? 200;
  for (const entry of entries.slice(0, shown)) {
    const parts = model.partsOf(entry.key);
    const removed = entry.state === 'removed';
    const tr = element('tr', `entry${removed ? ' removed' : ''}`);
    tr.dataset.unfold = entry.entry;

    const ordering = of(parts, 'index')?.text ?? 'unknown';
    const index = element('td', 'col-index');
    index.append(element('span', `ordering ordering-${ordering}`, ordering));
    const origin = of(parts, 'origin');
    const edition = of(parts, 'edition');
    if ((ordering === 'history' || ordering === 'coverage') && edition) {
      const hex = origin ? origin.text.replace(/^origin:/, '') : '';
      const version = element('span', 'version', `${hex.slice(0, 8)}${edition.text}`);
      version.title = `revision origin:${hex} ${edition.text}`;
      index.append(version);
    }
    tr.append(index);

    const entity = of(parts, 'entity');
    const ent = element('td', 'col-ent', entity ? trunc(entity.text, 18) : '');
    if (entity) ent.title = entity.text;
    tr.append(ent);
    tr.append(element('td', 'col-attr', of(parts, 'attribute')?.text ?? ''));

    const value = element('td', 'col-val');
    const spill = model.spills.get(entry.entry);
    const blob = model.blobs.get(entry.entry);
    const shownValue = of(parts, 'value');
    if (removed) value.textContent = '(retracted)';
    else if (shownValue) {
      const text = element('span', 'val', trunc(shownValue.text, 40));
      text.title = shownValue.text;
      value.append(text);
    } else {
      const notes = [];
      if (blob) notes.push(self.size(blob.size));
      const claim = model.claims.get(entry.entry);
      if (claim?.supersedes > 0) notes.push(`covers ${claim.supersedes}`);
      if (spill) notes.push(`→ ${of(parts, 'vtype')?.text ?? 'value'} ${spill.replace(/^asset:/, '').slice(0, 10)}`);
      value.textContent = notes.join(' · ');
    }
    tr.append(value);
    if (buffered) {
      const rides = model.buffers.get(entry.entry);
      const at = spans.find((candidate) => candidate.span === rides)?.at;
      tr.append(element('td', 'col-span', at === undefined ? '' : String(at)));
    }
    tbody.append(tr);

    if (!state.unfolded.has(entry.entry)) continue;
    const unfolded = element('tr', 'unfolded');
    const cell = element('td');
    cell.colSpan = buffered ? 5 : 4;
    const box = element('div', 'entry-detail');
    const vtype = of(parts, 'vtype');
    if (vtype) box.append(kv('type', vtype.text));
    if (entity) box.append(kv('entity', entity.text));
    const attribute = of(parts, 'attribute');
    if (attribute) box.append(kv('attribute', attribute.text));
    if (shownValue) box.append(kv('value', shownValue.text));
    box.append(kv('index', ordering));
    box.append(kv('state', entry.state));
    box.append(kv('rank', entry.rank));
    const version = model.versions.get(entry.entry);
    if (version) {
      box.append(kv('origin', version.origin));
      box.append(kv('edition', version.edition));
    }
    const claim = model.claims.get(entry.entry);
    if (claim) {
      for (const name of ['cause', 'collapsed', 'supersedes']) {
        if (claim[name] > 0) box.append(kv(name, claim[name]));
      }
      if (claim.retraction) box.append(kv('retraction', 'yes'));
    }
    if (blob) {
      box.append(kv('blob', blob.blob));
      box.append(kv('blob size', self.size(blob.size)));
    }
    if (spill) {
      box.append(kv('spilled to', spill));
      const read = model.values.get(spill);
      if (!read) {
        // Read only when asked: a value can be large, and may be fetched.
        const button = element('button', 'button', state.pending.get(spill) === 'open' ? 'reading…' : 'read value');
        button.type = 'button';
        button.dataset.value = spill;
        box.append(button);
      } else {
        const bytes = Uint8Array.from(read.bytes.slice(0, 4096));
        let text;
        try {
          text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
        } catch {
          text = read.bytes.slice(0, 1024).map((byte) => byte.toString(16).padStart(2, '0')).join(' ');
        }
        if (read.bytes.length > bytes.length) text += '\n…';
        box.append(kv('value size', self.size(read.size)));
        box.append(element('pre', 'value', text));
      }
    }
    box.append(element('div', 'k', 'key'));
    box.append(self.chips(parts));
    cell.append(box);
    unfolded.append(cell);
    tbody.append(unfolded);
  }
  const wrap = element('div', 'table');
  wrap.append(table);
  if (entries.length > shown) {
    const more = element('button', 'button', `show ${Math.min(200, entries.length - shown)} more of ${entries.length - shown}`);
    more.type = 'button';
    more.dataset.more = node;
    wrap.append(more);
  }
  return wrap;
}
