// Decode a key (`dialog/key-part`) or a span's separator
// (`dialog/separator-part`) into its components, once per distinct run
// of bytes: `[{ at, kind, text }]` in the key's own order. Answers what
// is already known and asks for the rest; the outline is drawn again
// when the answer arrives.
(self, formula, bytes) => {
  const state = self.__tree;
  if (!bytes.length) return [];
  const id = formula + ':' + bytes.map((byte) => byte.toString(16).padStart(2, '0')).join('');
  const known = state.parts.get(id);
  if (known) return known.parts;
  const entry = { parts: null };
  state.parts.set(id, entry);
  self.step(formula, { of: bytes }, ['at', 'kind', 'text'])
    .then((rows) => {
      entry.parts = rows
        .map((row) => ({ at: Number(row.at ?? 0), kind: String(row.kind ?? ''), text: String(row.text ?? '') }))
        .sort((a, b) => a.at - b.at);
    })
    .catch(() => { entry.parts = []; })
    .then(() => self.schedule());
  return null;
}
