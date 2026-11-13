// Load `item`'s children when it is first expanded. `lower` is the node's
// own lower bound, which its first child's key is dimmed against; each
// later child's is its previous sibling's key, and each is dimmed against
// its next sibling's as well. The dot spins while the children (possibly
// a fetch from the remote) are on their way. Expanding fetches the node
// itself too, so its row is read again after.
(self, item, hash, lower) => {
  const finish = () => {
    item.removeAttribute('lazy');
    item.removeAttribute('data-loading');
    // A fetched leaf has no children, and the tree item would spin forever.
    item.loading = false;
  };
  item.addEventListener('wa-lazy-load', async (event) => {
    event.stopPropagation();
    const state = self.__tree;
    if (!state) return;
    item.setAttribute('data-loading', '');
    let children;
    try {
      children = (await self.query('tree/child', { hash })).map((row) => self.node(row));
    } catch (error) {
      console.error(`tree/child: ${error?.message ?? error}`);
      finish();
      return;
    }
    if (self.__tree !== state) return;
    const kit = self.kit();
    const hashes = children.map((child) => {
      kit.put(state, child);
      return child.hash;
    });
    state.children.set(hash, hashes);
    const bounds = hashes.map((child) => state.nodes.get(child)?.bound ?? null);
    hashes.forEach((child, index) => {
      const prev = index === 0 ? lower : bounds[index - 1];
      item.append(self.item(child, prev, bounds[index + 1] ?? null));
    });
    finish();
    await self.refresh(item, hash);
  });
}
