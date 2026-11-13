// Load the segment `hash`'s entries into `pane`, as a table of Index,
// Entity, Attribute and Value; a row unfolds into its detail on a click.
//
// The index comes first because a segment holds more than facts: history
// and coverage records and blob references carry an entity, attribute and
// value of their own, and only the index says which kind a row is. A
// history or coverage record is told apart by the revision that wrote it,
// so its version, `<origin>@<edition>`, sits by the chip: two records at
// one edition under different origins are concurrent writes. A row with
// no value in its key says what it has instead: a blob's size, how much a
// coverage record covers, the block a spilled value lives in.
(self, pane, hash) => {
  const kit = self.kit();
  const state = self.__tree;
  const box = kit.el('div', 'entries');
  box.append(kit.el('div', 'status', 'loading entries…'));
  pane.append(box);
  const hex = (text) => (text.startsWith('0x') ? text.slice(2) : text);
  const kv = (key, value) => {
    const row = kit.el('div', 'kv');
    row.append(kit.el('span', 'k', key), kit.el('span', 'v', value));
    return row;
  };
  const ordering = (entry) => {
    const cell = kit.el('td', 'col-index');
    if (!entry.ordering) return cell;
    const chip = kit.el('span', `ordering ordering-${entry.ordering}`, entry.ordering);
    chip.title = `${entry.ordering} index`;
    cell.append(chip);
    if ((entry.ordering === 'history' || entry.ordering === 'coverage') && entry.edition !== null) {
      const version = kit.el('span', 'version');
      if (entry.origin) {
        const origin = hex(entry.origin);
        version.textContent = `${[...origin].slice(0, 8).join('')}@${entry.edition}`;
        version.title = `revision origin:${origin} @${entry.edition}`;
      } else {
        version.textContent = `@${entry.edition}`;
        version.title = `revision edition ${entry.edition}`;
      }
      cell.append(version);
    }
    return cell;
  };
  const detail = (entry) => {
    const out = kit.el('div', 'entry-detail');
    if (entry.type) out.append(kv('type', entry.type));
    if (entry.entity) out.append(kv('entity', entry.entity.startsWith('#') ? entry.entity.slice(1) : entry.entity));
    if (entry.attribute) out.append(kv('attribute', entry.attribute));
    if (entry.value !== undefined) out.append(kv('value', self.format(entry.value, entry.type ?? '')));
    if (entry.ordering) out.append(kv('index', entry.ordering));
    if (entry.origin) out.append(kv('origin', hex(entry.origin)));
    if (entry.edition !== null) out.append(kv('edition', String(entry.edition)));
    for (const [label, value] of [['cause', entry.cause], ['collapsed', entry.collapsed], ['supersedes', entry.supersedes]]) {
      if (value > 0) out.append(kv(label, String(value)));
    }
    if (entry.retraction) out.append(kv('retraction', 'yes'));
    if (entry.spill) out.append(kv('spilled to', entry.spill));
    if (entry.blob) {
      out.append(kv('blob', entry.blob.startsWith('#') ? entry.blob.slice(1) : entry.blob));
      if (entry.blobSize !== null) out.append(kv('blob size', kit.human(entry.blobSize)));
      if (entry.blobVersion !== null) out.append(kv('blob record version', String(entry.blobVersion)));
    }
    const key = kit.el('div', 'keybytes');
    self.key(key, entry.keyParts);
    out.append(kit.el('div', 'k', 'key'), key);
    return out;
  };
  const table = (entries) => {
    const table = kit.el('table');
    const head = kit.el('tr');
    for (const name of ['Index', 'Entity', 'Attribute', 'Value']) head.append(kit.el('th', null, name));
    const thead = kit.el('thead');
    thead.append(head);
    const body = kit.el('tbody');
    for (const entry of entries) {
      const row = kit.el('tr', entry.retracted ? 'entry removed' : 'entry');
      const entity = kit.el('td', 'col-ent');
      if (entry.entity) {
        entity.textContent = kit.trunc(entry.entity, 18);
        entity.title = entry.entity;
      }
      const attribute = kit.el('td', 'col-attr', entry.attribute ?? '');
      const value = kit.el('td', 'col-val');
      if (entry.retracted) value.textContent = '(retracted)';
      else if (entry.value !== undefined && entry.type) {
        const formatted = self.format(entry.value, entry.type);
        const span = kit.el('span', `val val-${entry.type.toLowerCase()}`, kit.trunc(formatted, 40));
        span.title = formatted;
        value.append(span);
      } else {
        const notes = [];
        if (entry.blobSize !== null) notes.push(kit.human(entry.blobSize));
        if (entry.supersedes > 0) notes.push(`covers ${entry.supersedes}`);
        if (entry.spill) notes.push(`→ ${entry.type ?? 'value'} ${kit.short(entry.spill, 10)}`);
        if (notes.length > 0) value.textContent = notes.join(' · ');
      }
      row.append(ordering(entry), entity, attribute, value);
      const unfold = kit.el('tr', 'detail');
      unfold.hidden = true;
      const cell = kit.el('td');
      cell.setAttribute('colspan', '4');
      cell.append(detail(entry));
      unfold.append(cell);
      row.addEventListener('click', () => { unfold.hidden = !unfold.hidden; });
      body.append(row, unfold);
    }
    table.append(thead, body);
    return table;
  };
  self.query('tree/entry', { hash }).then(
    (rows) => {
      // The selection may have moved on while the entries loaded.
      if (self.__tree !== state || state.selected !== hash) return;
      const entries = rows.map((row) => self.entry(row));
      box.replaceChildren(kit.el('div', 'k', `${entries.length} entries`), table(entries));
    },
    (error) => {
      if (self.__tree !== state || state.selected !== hash) return;
      box.replaceChildren(kit.el('div', 'err', error?.message ?? String(error)));
    },
  );
}
