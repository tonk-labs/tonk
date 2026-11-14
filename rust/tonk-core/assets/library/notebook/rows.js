// The blocks the rows carry now, in document order, or null when there is
// nothing to project yet.
//
// Sources come from the block rows, one per block entity, in whatever
// order they landed; order comes from the notebook's own `block` sequence,
// one `.notebook-order__item` per entry in position order with the entry's
// key. The block rows are a directory of every block in the space, so the
// ones of another notebook are dropped here: editing this one must never
// rewrite blocks it does not own.
(self) => {
  const state = self.__notebook;
  const notebook = self.dataset.notebook ?? null;
  const sources = new Map();
  const arrival = [];
  for (const row of self.querySelectorAll('.notebook-block-row')) {
    const { block: entity, source } = row.dataset;
    if (entity === undefined || source === undefined) continue;
    if (notebook !== null && row.dataset.notebook !== notebook) continue;
    if (!sources.has(entity)) arrival.push(entity);
    sources.set(entity, source);
  }
  // No rows before the first projection is "not loaded yet"; after it, a
  // notebook whose every block was deleted, still worth projecting.
  if (!sources.size && !state.projectedOnce) return null;
  const ordered = [];
  const placed = new Set();
  for (const entry of self.querySelectorAll('.notebook-order__item')) {
    const { block: entity, key } = entry.dataset;
    if (entity === undefined || key === undefined) continue;
    // An entry whose row has not landed is skipped, not shown blank.
    if (!sources.has(entity) || placed.has(entity)) continue;
    placed.add(entity);
    ordered.push({ entity, source: sources.get(entity), key });
  }
  // A block whose placement never landed still shows, at the end, and is
  // keyed on the next commit.
  for (const entity of arrival) {
    if (!placed.has(entity)) ordered.push({ entity, source: sources.get(entity), key: null });
  }
  return ordered;
}
