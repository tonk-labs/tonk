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
      'version': [`${at}.node/version`, 'UnsignedInteger'],
      'fanout-exponent': [`${at}.node/fanout-exponent`, 'UnsignedInteger'],
      'max-separator': [`${at}.node/max-separator`, 'UnsignedInteger'],
      'inline-n': [`${at}.node/inline-n`, 'UnsignedInteger'],
      'spill-prefix': [`${at}.node/spill-prefix`, 'UnsignedInteger'],
      'max-segment': [`${at}.node/max-segment`, 'UnsignedInteger'],
      'frame-ceiling-factor': [`${at}.node/frame-ceiling-factor`, 'UnsignedInteger'],
      'anchor-selector': [`${at}.node/anchor-selector`, 'UnsignedInteger'],
      'entry-overhead': [`${at}.node/entry-overhead`, 'UnsignedInteger'],
      'key-overhead': [`${at}.node/key-overhead`, 'UnsignedInteger'],
      'link-overhead': [`${at}.node/link-overhead`, 'UnsignedInteger'],
    },
    held: {
      local: [`${at}.node/held`, 'Boolean', session],
    },
    spill: {
      value: [`${at}.spill/value`, 'Entity'],
    },
    value: {
      size: [`${at}.value/size`, 'UnsignedInteger'],
      bytes: [`${at}.value/bytes`, 'Bytes'],
    },
    span: {
      parent: [`${at}.span/parent`, 'Entity'],
      at: [`${at}.span/at`, 'UnsignedInteger'],
      child: [`${at}.span/child`, 'Entity'],
      separator: [`${at}.span/separator`, 'Entity'],
      scale: [`${at}.span/scale`, 'UnsignedInteger'],
      novelty: [`${at}.span/novelty`, 'UnsignedInteger'],
      rank: [`${at}.span/rank`, 'UnsignedInteger'],
    },
    entry: {
      segment: [`${at}.entry/segment`, 'Entity'],
      at: [`${at}.entry/at`, 'UnsignedInteger'],
      key: [`${at}.entry/key`, 'Entity'],
      state: [`${at}.entry/state`, 'Text'],
    },
    rank: {
      rank: [`${at}.entry/rank`, 'UnsignedInteger'],
    },
    claim: {
      retraction: [`${at}.claim/retraction`, 'Boolean'],
      cause: [`${at}.claim/cause`, 'UnsignedInteger'],
      collapsed: [`${at}.claim/collapsed`, 'UnsignedInteger'],
      supersedes: [`${at}.claim/supersedes`, 'UnsignedInteger'],
    },
    version: {
      origin: [`${at}.version/origin`, 'Bytes'],
      edition: [`${at}.version/edition`, 'UnsignedInteger'],
    },
    buffer: {
      span: [`${at}.buffer/span`, 'Entity'],
    },
    blob: {
      blob: [`${at}.blob/blob`, 'Entity'],
      size: [`${at}.blob/size`, 'UnsignedInteger'],
    },
    part: {
      at: [`${at}.part/at`, 'UnsignedInteger'],
      kind: [`${at}.part/kind`, 'Text'],
      text: [`${at}.part/text`, 'Text'],
    },
  };
}
