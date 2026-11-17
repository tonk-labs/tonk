// The concepts the page reads, as `query` takes them: field name to
// attribute, type and, for state kept in the session, its layer. They are
// tree-inspector.yaml's, field for field.
(self) => {
  const session = { in: 'session:' };
  const at = 'xyz.tonk.tree-inspector';
  return {
    branch: {
      name: ['dialog.branch/name', 'Text'],
      tree: ['dialog.branch/tree', 'Entity'],
    },
    exploration: {
      started: [`${at}.exploration/started`, 'Float', session],
    },
    opened: {
      open: [`${at}.exploration/open`, 'Entity', { ...session, cardinality: 'many' }],
    },
    node: {
      kind: [`${at}.node/kind`, 'Text'],
      size: [`${at}.node/size`, 'UnsignedInteger'],
      count: [`${at}.node/count`, 'UnsignedInteger'],
      scale: [`${at}.node/scale`, 'UnsignedInteger'],
      novelty: [`${at}.node/novelty`, 'UnsignedInteger'],
    },
    span: {
      parent: [`${at}.span/parent`, 'Entity'],
      at: [`${at}.span/at`, 'UnsignedInteger'],
      child: [`${at}.span/child`, 'Entity'],
      separator: [`${at}.span/separator`, 'Entity'],
      scale: [`${at}.span/scale`, 'UnsignedInteger'],
      novelty: [`${at}.span/novelty`, 'UnsignedInteger'],
    },
    entry: {
      segment: [`${at}.entry/segment`, 'Entity'],
      at: [`${at}.entry/at`, 'UnsignedInteger'],
      key: [`${at}.entry/key`, 'Entity'],
      state: [`${at}.entry/state`, 'Text'],
      rank: [`${at}.entry/rank`, 'UnsignedInteger'],
    },
    part: {
      at: [`${at}.part/at`, 'UnsignedInteger'],
      kind: [`${at}.part/kind`, 'Text'],
      text: [`${at}.part/text`, 'Text'],
    },
  };
}
