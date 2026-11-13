// Commit the editor's document: split it into blocks, diff them against
// the base, and write every change in one transaction.
//
// Runs on the editor's debounced `change`, on leaving the editor, and
// before a projection that finds the author typed. The diff keeps each
// commit to what changed, however often it runs.
(self) => {
  const state = self.__notebook;
  // Never before the rows were projected (the base is empty, so every
  // block would read as new), nor before the editor mounted (its value is
  // still the buffered projection).
  if (!state?.prose || !state.projectedOnce || !state.prose.editor) return;
  // A draft has no notebook to write to: naming it creates one, and what
  // was typed goes with it.
  if (state.draft) return;
  const document = self.live();
  if (document === null || document === state.projected) return;
  // An earlier commit's inserts are on their way; the projection that sees
  // them land commits what was typed since.
  if (state.pending.inserts.length) return;

  const blocks = self.blocks();
  const next = blocks.split(document);
  const base = state.blocks.map((block) => ({ ...block }));
  const edit = blocks.reconcile(base, next);
  const moves = self.positions().moves(base, edit);
  // The heading is the title: an edit that changes it renames the notebook.
  const heading = blocks.titleOf(document);
  const title = heading !== null && heading !== state.title ? heading : null;
  const notebook = self.dataset.notebook ?? self.dataset.subject ?? 'id:notebook/scratch';
  const text = self.notation().commit(notebook, base, edit, title);

  const until = Date.now() + 8000;
  const { pending } = state;
  for (const [entity, source] of edit.changed) pending.edits.set(entity, { source, until });
  for (const [entity, key] of moves) pending.keys.set(entity, { key, until });
  for (const entity of edit.removed) pending.removed.set(entity, until);
  if (edit.created.length) {
    pending.inserts = edit.created.slice();
    pending.insertsUntil = until;
  }
  // The new base is the document as written, minus the blocks the inserts
  // have yet to produce.
  state.blocks = edit.order.flatMap((entity, index) => {
    if (entity === null) return [];
    const moved = moves.find(([held]) => held === entity);
    const key = moved ? moved[1] : (base.find((block) => block.entity === entity)?.key ?? null);
    return [{ entity, source: next[index], key }];
  });
  state.projected = document;
  if (title !== null) state.title = title;

  if (text === null) return;
  self.write(text);
  // Should the inserts never show, stop waiting for them once they expire.
  if (edit.created.length) self.later(() => self.project(), 8050);
}
