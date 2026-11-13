// Bring the editor in line with the rows: the first render, a commit of
// ours landing, and every other replica's edit.
//
// The rows are read through this notebook's writes that have not landed
// (`overlay`), so a projection never undoes what was just typed. When the
// author has typed since the last sync, that typing is committed first,
// against the base it came from, and the rows are projected over it: their
// text stays, and another replica's edit still lands.
(self) => {
  const state = self.__notebook;
  const blocks = self.blocks();
  const rows = self.rows();
  if (!rows) return;
  // A block whose entry did not render this time keeps the key it had.
  blocks.carryKeys(rows, state.blocks);
  blocks.sortByKey(rows);
  // A commit's inserted blocks have not reached the rows: projecting now
  // would drop them, and diffing against a base without them would insert
  // them twice.
  if (!self.land(rows)) return;

  const live = self.live() ?? '';
  const typed = state.projectedOnce && !state.draft && !!state.prose.editor && live !== state.projected;
  if (typed) {
    self.commit();
    if (state.pending.inserts.length) return;
  }

  const stored = self.overlay(rows);
  const now = new Map(stored.map((block) => [block.entity, block.source]));
  blocks.respell(stored, state.seen, state.blocks);
  state.seen = now;
  const document = blocks.project(stored.map((block) => block.source));
  state.blocks = stored;
  state.projectedOnce = true;
  // `value`, which patches only the span that differs and leaves the caret.
  if (document !== (self.live() ?? '')) state.prose.value = document;
  self.settle();
}
