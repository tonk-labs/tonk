// The roots to draw: the tree of each branch this session is on. The
// session names its branches, and a branch names its tree by the root
// node's `node:` entity.
async (self) => {
  const state = self.__tree;
  const variable = (name) => ({ '?': { name } });
  try {
    const sessions = await self.query({
      predicate: { with: {
        branch: { the: 'dialog.session/branch', as: 'Entity', cardinality: 'many' },
      } },
      terms: { this: variable('this'), branch: variable('branch') },
    });
    const mine = new Set(sessions.map((row) => String(row?.fields?.branch ?? '')).filter(Boolean));
    const branches = await self.query({
      predicate: { with: {
        name: { the: 'dialog.branch/name', as: 'Text', cardinality: 'one' },
        tree: { the: 'dialog.branch/tree', as: 'Entity', cardinality: 'one' },
      } },
      terms: { this: variable('this'), name: variable('name'), tree: variable('tree') },
    });
    const roots = branches
      .map((row) => ({
        branch: String(row?.this ?? ''),
        name: String(row?.fields?.name ?? ''),
        node: String(row?.fields?.tree ?? ''),
      }))
      .filter((root) => root.node.startsWith('node:'))
      .filter((root) => mine.size === 0 || mine.has(root.branch))
      .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    const before = state.roots.map((root) => root.node).join(' ');
    state.roots = roots;
    state.error = null;
    // A root that moved is a new node: read it, and open it as the old
    // one was. Subtrees the commit left alone keep their entities, so
    // what was open under them stays open.
    for (const root of roots) {
      if (!state.nodes.has(root.node)) self.open(root.node);
    }
    if (before !== roots.map((root) => root.node).join(' ')) self.schedule();
  } catch (error) {
    state.error = String(error?.message ?? error);
    self.schedule();
  }
}
