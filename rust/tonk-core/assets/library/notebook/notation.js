// Notation: the documents a notebook evaluates to write itself.
//
// One commit is ONE document, so it lands whole or not at all: the block
// edits, the moves, the removals, a rename and the inserts of an edit
// (`blocks().reconcile`) together.
//
// New blocks name no entity. A block's identity derives from its
// `block/insert` body, the way every anchor-less assertion's does, and the
// library's rules give it a position from the block it precedes (`next`)
// and the existing block its run follows (`prev`). A run of new blocks is
// written back to front, so each `next: ?b<n>` names a variable an earlier
// assertion bound.
//
// Pure: no DOM, no state. Returns the functions, evaluated once per tag.
(self) => (self.constructor.__notebookNotation ??= (() => {
  // A text value as a double-quoted scalar: JSON's string syntax is YAML's,
  // so indentation, trailing spaces, tabs and a leading `?` all survive.
  const quote = (text) => JSON.stringify(text);

  // One `block/insert!` per created block (`order` holds null where each
  // goes), or null when nothing was created. Adjacent new blocks chain by
  // variable; the run's every block carries the existing block before it,
  // or `tonk:notebook/edge` at the head.
  const inserts = (notebook, order, created) => {
    if (!created.length) return null;
    const slots = [];
    let next = 0;
    order.forEach((slot, index) => {
      if (slot === null && next < created.length) slots.push([index, created[next++]]);
    });
    if (!slots.length) return null;
    const runs = [];
    for (const slot of slots) {
      const run = runs[runs.length - 1];
      if (run && run[run.length - 1][0] + 1 === slot[0]) run.push(slot);
      else runs.push([slot]);
    }
    let out = '';
    let nth = 0;
    for (const run of runs) {
      const base = nth;
      const previous = order.slice(0, run[0][0]).reverse().find((slot) => slot !== null) ?? null;
      const vars = run.map((_, offset) => `b${base + offset}`);
      for (let offset = run.length - 1; offset >= 0; offset--) {
        const [index, source] = run[offset];
        out += 'block/insert!:\n';
        out += `  this: ?${vars[offset]}\n`;
        out += `  notebook: ${notebook}\n`;
        out += `  source: ${quote(source)}\n`;
        const following = order.slice(index + 1).find((slot) => slot !== null) ?? null;
        if (offset + 1 < run.length) out += `  next: ?${vars[offset + 1]}\n`;
        else if (following !== null) out += `  next: ${following}\n`;
        else out += '  next: case:none\n';
        out += `  prev: ${previous ?? 'tonk:notebook/edge'}\n\n`;
        nth += 1;
      }
    }
    return out || null;
  };

  // Everything one edit writes, as one document, or null when it writes
  // nothing. `stored` is what the edit was diffed against: the keys a move
  // leaves and a removal names. `title` is the new title, when the heading
  // changed.
  const commit = (notebook, stored, edit, title) => {
    const keyOf = (entity) => stored.find((block) => block.entity === entity)?.key ?? null;
    let out = '';
    for (const [entity, source] of edit.changed) {
      out += `block/edit!:\n  subject: ${entity}\n  notebook: ${notebook}\n  source: ${quote(source)}\n\n`;
    }
    for (const [entity, key] of self.positions().moves(stored, edit)) {
      // A block that has an entry MOVES, its old entry going in the same
      // commit; one that never had an entry is placed.
      const from = keyOf(entity);
      out += from === null
        ? `block/place!:\n  subject: ${entity}\n  notebook: ${notebook}\n`
        : `block/move!:\n  subject: ${entity}\n  notebook: ${notebook}\n  from: ${quote(from)}\n`;
      out += `  key: ${quote(key)}\n\n`;
    }
    for (const entity of edit.removed) {
      // A block whose entry never landed has no key; its own facts still go.
      out += `block/remove!:\n  subject: ${entity}\n  notebook: ${notebook}\n  key: ${quote(keyOf(entity) ?? '')}\n\n`;
    }
    if (title !== null && title !== undefined) {
      out += `notebook/retitle!:\n  subject: ${notebook}\n  title: ${quote(title)}\n\n`;
    }
    out += inserts(notebook, edit.order, edit.created) ?? '';
    return out || null;
  };

  // A draft's body as blocks, heading kept: split on blank lines.
  const draftBlocks = (body) => body.split('\n\n').map((chunk) => chunk.trim()).filter((chunk) => chunk !== '');

  // A source as a YAML literal block scalar.
  const blockScalar = (source) => {
    let out = '|-\n';
    const rows = source === '' ? [] : source.replace(/\n$/, '').split('\n');
    for (const line of rows) out += `    ${line.replace(/\r$/, '')}\n`;
    return out.trimEnd();
  };

  // Creating a notebook from the index's switcher: the title, and the
  // draft's blocks (at least one, an empty one for a title-only create, so
  // the notebook has a block to satisfy its concept), in one commit.
  const create = (entity, title, body) => {
    let out = `notebook/named!:\n  this: ${entity}\n  title: ${quote(title)}\n\n`;
    const sources = draftBlocks(body);
    if (!sources.length) sources.push('');
    for (let index = sources.length - 1; index >= 0; index--) {
      out += 'block/insert!:\n';
      out += `  this: ?b${index}\n`;
      out += `  notebook: ${entity}\n`;
      out += `  source: ${blockScalar(sources[index])}\n`;
      out += index + 1 < sources.length ? `  next: ?b${index + 1}\n` : '  next: case:none\n';
      out += '  prev: tonk:notebook/edge\n\n';
    }
    return out;
  };

  return Object.freeze({ quote, inserts, commit, draftBlocks, blockScalar, create });
})())
