// Take the blocks our pending inserts became into the base, once they all
// show in `rows`; false while still waiting.
//
// Matched by source: their entities derive on the worker, and nothing
// commits while inserts are out, so their sources have not changed since.
// Until the wait expires an insert has landed only once its entry shows
// too; after it, its row alone will do.
(self, rows) => {
  const state = self.__notebook;
  const { pending } = state;
  if (!pending.inserts.length) return true;
  const expired = pending.insertsUntil <= Date.now();
  const landed = self.blocks().landedInserts(rows, state.blocks, pending.inserts, expired);
  if (landed.length < pending.inserts.length && !expired) return false;
  pending.inserts = [];
  // Each goes into the base after the nearest block before it in the rows,
  // so the base keeps the editor's order.
  const base = state.blocks;
  rows.forEach((block, index) => {
    if (!landed.some((held) => held.entity === block.entity)) return;
    let after = -1;
    for (let previous = index - 1; previous >= 0 && after === -1; previous--) {
      after = base.findIndex((held) => held.entity === rows[previous].entity);
    }
    base.splice(after + 1, 0, { ...block });
  });
  return true;
}
