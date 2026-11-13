// Ask the host for the rows of one of the worker's `tree/*` formulas,
// with `terms` (`{hash}`, or nothing for the root), routed as this element
// names. Rejects when no host answers or the query fails.
async (self, formula, terms) => {
  const detail = { ...(self.__tree?.route ?? {}), query: { predicate: formula, terms } };
  const event = new CustomEvent('tonk-query', {
    detail, bubbles: true, composed: true, cancelable: true,
  });
  self.dispatchEvent(event);
  if (!event.defaultPrevented || !detail.result) throw new Error('no host answered tonk-query');
  try {
    const rows = await detail.result;
    return Array.isArray(rows) ? rows : [];
  } catch (error) {
    throw new Error(`${formula}: ${error?.message ?? String(error)}`);
  }
}
