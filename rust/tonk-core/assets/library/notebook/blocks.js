// Blocks: projecting a notebook's ordered blocks into the one markdown
// document the editor shows, and recovering them from an edited document.
//
// A notebook stores blocks, not one string: a keystroke writes the block
// it changed, not every block. The document is a projection, and
// `split(project(blocks))` gives the blocks back for any blocks `split`
// produced, so a load-edit-save cycle cannot reshape a document nobody
// touched.
//
// Boundaries are blank lines, which is what prosemirror-markdown puts
// between top-level blocks, except inside a fence: a dialog cell's body is
// full of blank lines and every one of them is the cell's. A heading rides
// with the first chunk under it, so a section moves as one.
//
// A block is `{ entity, source, key }`: `key` is its position in the
// notebook's `block` sequence, a string that sorts in byte order, or null
// for a block whose entry has not landed.
//
// Pure: no DOM, no state. Returns the functions, evaluated once per tag.
(self) => (self.constructor.__notebookBlocks ??= (() => {
  // `str::lines`: split on `\n`, drop a `\r` before it, no final empty line.
  const lines = (text) => {
    if (text === '') return [];
    const parts = text.split('\n');
    if (parts[parts.length - 1] === '') parts.pop();
    return parts.map((line) => (line.endsWith('\r') ? line.slice(0, -1) : line));
  };
  const trimStartSpaces = (text) => text.replace(/^ +/, '');
  const run = (text, ch) => {
    let count = 0;
    while (count < text.length && text[count] === ch) count += 1;
    return count;
  };

  // The fence a line opens: three or more backticks or tildes, indented by
  // at most three spaces. Both kinds, since one may hold the other.
  const opening = (line) => {
    const trimmed = trimStartSpaces(line);
    if (line.length - trimmed.length > 3) return null;
    if (trimmed[0] === '`' && run(trimmed, '`') >= 3) return { ch: '`', length: run(trimmed, '`') };
    if (trimmed[0] === '~' && run(trimmed, '~') >= 3) return { ch: '~', length: run(trimmed, '~') };
    return null;
  };
  // Whether `line` closes `fence`: the same delimiter, at least as long,
  // and nothing else on the line.
  const closes = (fence, line) => {
    const trimmed = trimStartSpaces(line).trimEnd();
    const count = run(trimmed, fence.ch);
    return count >= fence.length && trimmed.length === count;
  };

  // The raw chunks: split at blank lines outside fences.
  const chunks = (document) => {
    const out = [];
    let current = [];
    let fence = null;
    for (const line of lines(document)) {
      if (fence) {
        current.push(line);
        if (closes(fence, line)) fence = null;
      } else if (line.trim() === '') {
        if (current.length) {
          out.push(current.join('\n'));
          current = [];
        }
      } else {
        fence = opening(line);
        current.push(line);
      }
    }
    if (current.length) out.push(current.join('\n'));
    return out;
  };

  // An ATX heading: one to six `#` and a space.
  const isHeading = (chunk) => {
    const trimmed = trimStartSpaces(chunk);
    const hashes = run(trimmed, '#');
    return hashes >= 1 && hashes <= 6 && trimmed.slice(hashes).startsWith(' ');
  };

  // Join ordered sources with one blank line between, dropping empty ones.
  const project = (sources) => sources
    .map((source) => source.replace(/^\n+|\n+$/g, ''))
    .filter((source) => source !== '')
    .join('\n\n');

  // Group chunks: consecutive headings attach to the first chunk after
  // them; the next heading after content opens a new block. `onBlock`
  // receives each block's first chunk index and chunk count.
  const group = (document, onBlock) => {
    let pending = [];
    let start = 0;
    chunks(document).forEach((chunk, index) => {
      if (isHeading(chunk)) {
        if (pending.some((held) => !isHeading(held))) {
          onBlock(start, pending);
          pending = [];
        }
        if (pending.length === 0) start = index;
        pending.push(chunk);
      } else if (pending.some(isHeading)) {
        pending.push(chunk);
        onBlock(start, pending);
        pending = [];
      } else {
        onBlock(index, [chunk]);
      }
    });
    if (pending.length) onBlock(start, pending);
  };

  // An edited document as blocks, each heading grouped with its content.
  const split = (document) => {
    const out = [];
    group(document, (_, held) => out.push(held.join('\n\n')));
    return out;
  };

  // The same grouping as `[first, count]` over the top-level nodes, so the
  // node the caret is in leads back to its block.
  const blockSpans = (document) => {
    const out = [];
    group(document, (start, held) => out.push([start, held.length]));
    return out;
  };
  const spanAt = (document, index) =>
    blockSpans(document).find(([start, length]) => index >= start && index < start + length) ?? null;

  // The title: the text of the leading ATX heading, or null when the
  // document does not open with one (it keeps the title it had).
  const titleOf = (document) => {
    const first = lines(document.trimStart())[0];
    if (first === undefined) return null;
    const trimmed = trimStartSpaces(first);
    const hashes = run(trimmed, '#');
    if (hashes < 1 || hashes > 6) return null;
    const text = trimmed.slice(hashes).trim().replace(/#+$/, '').trim();
    return text === '' ? null : text;
  };

  const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

  // Order by key; keyless blocks last, in the order they came.
  const sortByKey = (blocks) => blocks.sort((a, b) => {
    if (a.key !== null && b.key !== null) return compare(a.key, b.key);
    if (a.key !== null) return -1;
    if (b.key !== null) return 1;
    return 0;
  });

  // A row showing no entry keeps the key its block had in `base`: the
  // block rows and the order entries are separate renders, and a block
  // read as unplaced would be placed a second time at the end.
  const carryKeys = (rows, base) => {
    for (const row of rows) {
      if (row.key !== null) continue;
      row.key = base.find((block) => block.entity === row.entity)?.key ?? null;
    }
    return rows;
  };

  // The rows a commit's inserts became, matched by source among the rows
  // `base` does not hold, in the order of `inserts`. A row counts once its
  // entry shows too, unless `keyless`.
  const landedInserts = (rows, base, inserts, keyless) => {
    const fresh = rows
      .filter((row) => !base.some((block) => block.entity === row.entity))
      .filter((row) => keyless || row.key !== null);
    const landed = [];
    for (const source of inserts) {
      const at = fresh.findIndex((row) => row.source === source);
      if (at !== -1) landed.push(fresh.splice(at, 1)[0]);
    }
    return landed;
  };

  // Keep the editor's spelling of every block whose row is unchanged since
  // the last projection (`seen`, entity to source): the editor normalizes
  // what it is given, and projecting the stored spelling would rewrite it
  // into the editor on every projection.
  const respell = (stored, seen, base) => {
    for (const block of stored) {
      if (seen.get(block.entity) !== block.source) continue;
      const ours = base.find((held) => held.entity === block.entity);
      if (ours) block.source = ours.source;
    }
    return stored;
  };

  // How much of `a` survives in `b`: shared characters at the start plus
  // at the end, none counted twice.
  const likeness = (a, b) => {
    const left = Array.from(a);
    const right = Array.from(b);
    let prefix = 0;
    while (prefix < left.length && prefix < right.length && left[prefix] === right[prefix]) prefix += 1;
    const room = Math.min(left.length, right.length) - prefix;
    let suffix = 0;
    while (suffix < room && left[left.length - 1 - suffix] === right[right.length - 1 - suffix]) suffix += 1;
    return prefix + suffix;
  };

  // Diff an edited document's blocks (`next`, sources) against the stored
  // ones. Unchanged sources keep their entity wherever they moved, so a
  // cut and paste is a reorder; what is left pairs up between the same
  // verbatim anchors, so a typo fix keeps its block's identity.
  //
  // Returns `{ changed: [[entity, source]], created: [source], removed:
  // [entity], order: [entity | null], reordered }`, a null in `order`
  // standing for the next of `created`.
  const reconcile = (stored, next) => {
    const bySource = new Map();
    stored.forEach((block, index) => {
      const queue = bySource.get(block.source) ?? [];
      queue.push(index);
      bySource.set(block.source, queue);
    });
    const claimed = next.map(() => null);
    const taken = stored.map(() => false);
    next.forEach((source, slot) => {
      const queue = bySource.get(source);
      if (queue?.length) {
        const index = queue.shift();
        claimed[slot] = index;
        taken[index] = true;
      }
    });

    const anchors = [];
    claimed.forEach((index, slot) => { if (index !== null) anchors.push([slot, index]); });
    const gaps = [];
    let from = 0;
    let after = null;
    for (const [slot, index] of anchors) {
      gaps.push([from, slot, after, index]);
      from = slot + 1;
      after = index;
    }
    gaps.push([from, next.length, after, null]);

    for (const [begin, end, low, high] of gaps) {
      const fresh = [];
      for (let slot = begin; slot < end; slot++) fresh.push(slot);
      if (!fresh.length) continue;
      const spare = stored.map((_, index) => index).filter((index) => {
        if (taken[index]) return false;
        if (low !== null && high !== null) return low > high || (low < index && index < high);
        if (low !== null) return low < index;
        if (high !== null) return index < high;
        return true;
      });
      if (!spare.length) continue;
      if (fresh.length === 1 && spare.length === 1) {
        claimed[fresh[0]] = spare[0];
        taken[spare[0]] = true;
        continue;
      }
      const pairs = [];
      for (const slot of fresh) {
        for (const index of spare) {
          const score = likeness(stored[index].source, next[slot]);
          if (score > 0) pairs.push([score, slot, index]);
        }
      }
      pairs.sort((a, b) => b[0] - a[0]);
      for (const [, slot, index] of pairs) {
        if (claimed[slot] === null && !taken[index]) {
          claimed[slot] = index;
          taken[index] = true;
        }
      }
    }

    const edit = { changed: [], created: [], removed: [], order: [], reordered: false };
    next.forEach((source, slot) => {
      const index = claimed[slot];
      if (index === null) {
        edit.created.push(source);
        edit.order.push(null);
      } else {
        const block = stored[index];
        if (block.source !== source) edit.changed.push([block.entity, source]);
        edit.order.push(block.entity);
      }
    });
    stored.forEach((block, index) => { if (!taken[index]) edit.removed.push(block.entity); });
    edit.reordered = edit.order.length !== stored.length
      || edit.order.some((entity, index) => entity !== stored[index].entity);
    return edit;
  };

  return Object.freeze({
    project, split, blockSpans, spanAt, titleOf, sortByKey, carryKeys, landedInserts, respell, reconcile,
  });
})())
