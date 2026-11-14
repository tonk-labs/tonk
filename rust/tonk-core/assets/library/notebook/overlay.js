// The rows read through this notebook's writes that have not landed: each
// pending write shadows the row it targets until the row shows it, or it
// expires. Returns the rows to project.
//
// Another replica's write to a block ours is in flight for is shadowed
// too: ours lands after it and wins the block. Projecting theirs meanwhile
// would replace the text under the caret, and the next commit would put
// ours back. Two replicas typing in one block at once keep the later
// writer's text: a merge would need the version each started from, which
// the rows do not carry.
(self, stored) => {
  const { pending } = self.__notebook;
  const now = Date.now();
  for (const [entity, edit] of pending.edits) {
    const block = stored.find((held) => held.entity === entity);
    if (!block) {
      if (edit.until <= now) pending.edits.delete(entity);
    } else if (block.source === edit.source || edit.until <= now) {
      // Landed, or never will: the row is the truth again.
      pending.edits.delete(entity);
    } else {
      block.source = edit.source;
    }
  }
  for (const [entity, { key, until }] of pending.keys) {
    const block = stored.find((held) => held.entity === entity);
    if (!block) {
      if (until <= now) pending.keys.delete(entity);
    } else if (block.key === key || until <= now) {
      pending.keys.delete(entity);
    } else {
      block.key = key;
    }
  }
  for (const [entity, until] of pending.removed) {
    if (until <= now || !stored.some((held) => held.entity === entity)) pending.removed.delete(entity);
  }
  const kept = stored.filter((block) => !pending.removed.has(block.entity));
  // A pending re-key moves its block.
  return self.blocks().sortByKey(kept);
}
