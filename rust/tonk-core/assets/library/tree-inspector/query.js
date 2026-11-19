// One read of a concept through the host: `fields` maps each field to its
// attribute (`the`, `as`, and `in` for one kept in a layer), and a row's
// fields come back under the same names. Routed by the element's place in
// the page; rejects when no host answers.
(self, fields) => {
  const terms = { this: { '?': { name: 'this' } } };
  const descriptor = {};
  for (const [name, [the, as, more]] of Object.entries(fields)) {
    descriptor[name] = { the, as, cardinality: 'one', ...(more ?? {}) };
    terms[name] = { '?': { name } };
  }
  const detail = { query: { predicate: { with: descriptor }, terms } };
  const event = new CustomEvent('tonk-query', { detail, bubbles: true, composed: true, cancelable: true });
  self.dispatchEvent(event);
  if (!event.defaultPrevented || !detail.result) return Promise.reject(new Error('no host answered tonk-query'));
  return Promise.resolve(detail.result).then((rows) => (Array.isArray(rows) ? rows : []));
}
