// An entry of a segment, from a `tree/entry` row.
//
//   keyParts      its key as the worker decoded it, `{kind, text, hex}`.
//   ordering      the index the key belongs to: entity, attribute, value,
//                 history, coverage or blob. A segment holds more than
//                 facts, and only this says which kind a row is.
//   retracted     whether the fact is withdrawn.
//   entity, attribute, type, value
//                 the fact, where the key carries one.
//   origin, edition
//                 the claim's version: the replica that wrote it (hex) and
//                 its causal depth.
//   cause, collapsed, supersedes, retraction
//                 its lineage: prior versions in its cause, how many were
//                 folded into it, how many it covers.
//   spill         the block a value too big to inline lives in.
//   blob, blobSize, blobVersion
//                 for a blob row: the hash, its size, and its record's
//                 encoding version.
(self, row) => {
  const fields = row?.fields ?? {};
  const text = (key) => (typeof fields[key] === 'string' ? fields[key] : null);
  const natural = (key) => (Number.isInteger(fields[key]) && fields[key] >= 0 ? fields[key] : null);
  const list = fields['key-parts'];
  const keyParts = Array.isArray(list) && list.every((part) => part && typeof part.kind === 'string')
    ? list.map((part) => ({
      kind: part.kind,
      text: typeof part.text === 'string' ? part.text : '',
      hex: typeof part.hex === 'string' ? part.hex : '',
    }))
    : [];
  return {
    keyParts,
    retracted: fields.retracted === true,
    entity: text('entity'),
    attribute: text('attribute'),
    type: text('type'),
    value: Object.prototype.hasOwnProperty.call(fields, 'value') ? fields.value : undefined,
    ordering: text('ordering'),
    origin: text('origin'),
    edition: natural('edition'),
    cause: natural('cause'),
    collapsed: natural('collapsed'),
    supersedes: natural('supersedes'),
    retraction: fields.retraction === true,
    spill: text('spill'),
    blob: text('blob'),
    blobSize: natural('blob-size'),
    blobVersion: natural('blob-version'),
  };
}
