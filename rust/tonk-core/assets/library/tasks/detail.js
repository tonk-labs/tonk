async (self) => {
  // The parts an invocation may not have yet: which command it was, what
  // it is doing or produced, and the words for that. Read for every row on
  // each change, since any of them can change while it runs.
  const branch = self.getAttribute('with');
  const one = self.__one;
  const read = async (field, attribute, as, of) => {
    const found = await window.tonk.query({
      predicate: { with: { [field]: one(attribute, as) } },
      terms: { this: of, [field]: { '?': { name: field } } },
    }, { with: branch }).catch(() => null);
    const rows = Array.isArray(found) ? found : (found && found.rows) || [];
    return rows.length ? (rows[0].fields || {})[field] : undefined;
  };
  await Promise.all([...self.__rows.entries()].map(async ([id, fields]) => {
    const [concept, state, output, finished] = await Promise.all([
      read('concept', 'xyz.tonk.command/concept', 'Entity', id),
      read('state', 'xyz.tonk.invocation/state', 'Entity', id),
      read('output', 'xyz.tonk.invocation/output', 'Entity', id),
      read('finished', 'xyz.tonk.invocation/finished', 'Float', id),
    ]);
    const said = async (entity) => (entity
      ? read('status', 'xyz.tonk.state/status', 'Text', entity) : undefined);
    Object.assign(fields, {
      concept, state, output, finished,
      stateStatus: await said(state),
      outputStatus: await said(output),
    });
  }));
}
