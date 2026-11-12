// Draw the outline from the root, or `status` in its place: an empty
// tree, or why it could not be read.
(self, status) => {
  const kit = self.kit();
  const state = self.__tree;
  const tree = state.shadow.querySelector('wa-tree');
  if (!tree) return;
  if (status !== undefined) {
    tree.replaceChildren(kit.el('div', 'status', status));
    return;
  }
  tree.replaceChildren();
  if (state.root) tree.append(self.item(state.root, null, null));
}
