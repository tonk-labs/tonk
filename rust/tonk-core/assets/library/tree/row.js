// One outline row: the node's key dimmed against its siblings', then its
// size (a bar scaled to the widest node seen, and the bytes), its count,
// and the operations buffered on it. A node the replica has not fetched
// shows dimmed, with a cloud.
//
// Novelty is two numbers, each shown only when it counts something: `⊕n`
// what this node buffers for its own children (a segment never does), and
// `↓n` what its parent buffers against its span, work headed here that
// is not written down yet. Only the second is known for an unfetched node.
(self, node, prev, next) => {
  const kit = self.kit();
  const state = self.__tree;
  const row = kit.el('span', node.cached ? 'row' : 'row remote');
  const key = kit.el('span', 'keystr');
  key.title = node.hash;
  if (node.bound !== null && node.boundParts.length > 0) self.key(key, node.boundParts, { hex: node.bound, prev, next });
  else key.textContent = kit.short(node.hash, 8);
  row.append(key);

  const size = kit.el('span', 'col size');
  const fraction = Math.min(1, Math.max(0.02, node.size / Math.max(1, state.maxSize)));
  const bar = kit.el('span', 'sizebar');
  bar.style.width = `calc(70px * ${fraction})`;
  size.append(bar, kit.el('span', 'sizenum', kit.human(node.size)));
  row.append(size, kit.el('span', 'col count', String(node.count)));

  const novelty = kit.el('span', 'col novelty');
  if (node.novelty > 0) {
    const own = kit.el('span', 'nov nov-own', `⊕${node.novelty}`);
    own.title = `${node.novelty} ops buffered on this node`;
    novelty.append(own);
  }
  if (node.pending > 0) {
    const pending = kit.el('span', 'nov nov-pending', `↓${node.pending}`);
    pending.title = `${node.pending} ops buffered against this subtree in its parent`;
    novelty.append(pending);
  }
  row.append(novelty);

  if (!node.cached) {
    const cloud = kit.el('wa-icon', 'remote-icon');
    cloud.setAttribute('name', 'cloud');
    cloud.setAttribute('label', 'not cached locally');
    row.append(cloud);
  }
  return row;
}
