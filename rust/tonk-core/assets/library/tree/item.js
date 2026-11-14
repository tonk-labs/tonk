// The outline item for the node `hash`. `prev` and `next` are its
// siblings' separator keys, against which its own key is dimmed.
//
// A node expands when it is an index with children, or when the replica
// has not fetched it yet: its kind is unknown until expanding fetches it.
// The toggle is the node's dot, filled or hollow by locality, in place of
// the tree's chevron; a leaf's dot sits on the item, where the connector
// lines end.
(self, hash, prev, next) => {
  const kit = self.kit();
  const node = self.__tree.nodes.get(hash);
  const item = kit.el('wa-tree-item');
  item.setAttribute('data-hash', hash);
  if (!node) return item;
  const expandable = (node.kind === 'index' && node.count > 0) || !node.cached;
  if (expandable) {
    const expand = kit.dot(node.cached);
    expand.setAttribute('slot', 'expand-icon');
    const collapse = kit.dot(node.cached);
    collapse.setAttribute('slot', 'collapse-icon');
    item.append(expand, collapse);
  }
  item.append(self.row(node, prev, next));
  if (expandable) {
    item.setAttribute('lazy', '');
    // The parent's lower bound is its first child's lower bound too.
    self.expand(item, hash, prev);
  } else {
    item.append(kit.dot(node.cached, 'dot-leaf'));
  }
  return item;
}
