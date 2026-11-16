// One read through the host, routed by this element's place in the page.
// Rejects when no host answers.
(self, query) => {
  const detail = { query };
  const event = new CustomEvent('tonk-query', {
    detail, bubbles: true, composed: true, cancelable: true,
  });
  self.dispatchEvent(event);
  if (!event.defaultPrevented || !detail.result) {
    return Promise.reject(new Error('no host answered tonk-query'));
  }
  return Promise.resolve(detail.result).then((rows) => (Array.isArray(rows) ? rows : []));
}
