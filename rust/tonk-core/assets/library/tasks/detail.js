async (self) => {
  // What a row says beyond the invocation's own rows: what its command
  // answers to in the palette, and the words for what it is doing. Read
  // for every row on each change, since the state can change while it runs.
  const branch = self.getAttribute('with');
  const one = self.__one;
  const read = async (field, attribute, as, of, layer) => {
    const found = await window.tonk.query({
      predicate: { with: { [field]: { ...one(attribute, as), ...(layer ? { in: layer } : {}) } } },
      terms: { this: of, [field]: { '?': { name: field } } },
    }, { with: branch }).catch(() => null);
    const rows = Array.isArray(found) ? found : (found && found.rows) || [];
    return rows.length ? (rows[0].fields || {})[field] : undefined;
  };
  await Promise.all([...self.__rows.values()].map(async (fields) => {
    // Its `action:` name, so a row says "install" rather than the
    // concept's content-derived id.
    const [name, status] = await Promise.all([
      fields.of ? read('name', 'tonk.dialog.intent.action/name', 'Text', fields.of) : undefined,
      fields.state ? read('status', 'xyz.tonk.state/status', 'Text', fields.state, 'session:') : undefined,
    ]);
    Object.assign(fields, { name, status });
  }));
}
