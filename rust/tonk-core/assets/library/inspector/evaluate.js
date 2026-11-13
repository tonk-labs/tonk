// Evaluate `notation` against this inspector's branch through the host:
// a dry run with `transact` false, a commit with it true. Resolves to the
// worker's evaluate response; rejects with the host's message, or when no
// host answers.
(self, notation, transact) => {
  const detail = { document: notation, transact };
  const event = new CustomEvent('tonk-evaluate', {
    detail, bubbles: true, composed: true, cancelable: true,
  });
  self.dispatchEvent(event);
  if (!event.defaultPrevented || !detail.result) {
    return Promise.reject(new Error('no host answered tonk-evaluate'));
  }
  return Promise.resolve(detail.result).catch((error) => {
    throw new Error(error?.message ?? String(error));
  });
}
