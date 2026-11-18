// One read through the host, from `from` (this element by default), so
// it is routed by the element's place in the page, as the element's own
// definition was. Rejects when no host answers.
(self, query, from) => {
  const detail = { query };
  const event = new CustomEvent('tonk-query', {
    detail, bubbles: true, composed: true, cancelable: true,
  });
  (from ?? self).dispatchEvent(event);
  if (!event.defaultPrevented || !detail.result) {
    return Promise.reject(new Error('no host answered tonk-query'));
  }
  return Promise.resolve(detail.result);
}
