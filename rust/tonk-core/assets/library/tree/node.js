// A node from a `tree/node` or `tree/child` row.
//
//   hash        the node's `#<base58>`: a child row's `child`, else `this`.
//   kind        'index' (child links) or 'segment' (entries).
//   size, count its bytes, and its children or entries.
//   bound       its separator key as raw hex, for the routing pivot, and
//   boundParts  the same key as the worker decoded it, `{kind, text, hex}`.
//   rank        its seam rank, when its parent says.
//   cached      whether the replica holds it (false until fetched).
//   scale       the advisory log-scale size of its subtree.
//   novelty     operations it buffers for its own children.
//   pending     operations its PARENT buffers against its span: known for
//               a child whether or not the child is fetched.
//   manifest    the format it was written under, `[label, value]` in the
//               manifest's own reading order.
(self, row) => {
  const fields = row?.fields ?? {};
  const text = (key) => (typeof fields[key] === 'string' ? fields[key] : null);
  const natural = (key) => (Number.isInteger(fields[key]) && fields[key] >= 0 ? fields[key] : null);
  // All or nothing, as the decode is: a malformed part falls back to hex.
  const parts = (key) => {
    const list = fields[key];
    if (!Array.isArray(list) || !list.every((part) => part && typeof part.kind === 'string')) return [];
    return list.map((part) => ({
      kind: part.kind,
      text: typeof part.text === 'string' ? part.text : '',
      hex: typeof part.hex === 'string' ? part.hex : '',
    }));
  };
  const manifest = fields.manifest && typeof fields.manifest === 'object' ? fields.manifest : {};
  return {
    hash: text('child') ?? String(row?.this ?? ''),
    kind: text('kind') === 'segment' ? 'segment' : 'index',
    size: natural('size') ?? 0,
    count: natural('count') ?? 0,
    bound: text('bound'),
    boundParts: parts('bound-parts'),
    rank: natural('rank'),
    cached: fields.cached !== false,
    scale: natural('scale'),
    novelty: natural('novelty'),
    pending: natural('pending'),
    manifest: ['version', 'fanout', 'max-separator', 'inline', 'spill-prefix', 'max-segment']
      .filter((key) => Number.isInteger(manifest[key]) && manifest[key] >= 0)
      .map((key) => [key, manifest[key]]),
  };
}
