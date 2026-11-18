// Follow the state the page is drawn from: the explorations and what is
// open in them, what the replica holds, and the branch's root. A frame of either subscription is
// only a signal to read everything again. Dispatched from a consumer of
// its own, which carries the `reset`/`update` methods the host calls; a
// host still starting may not claim the event yet, so it is offered again
// a few times.
(self) => {
  const state = self.__tree;
  if (state.watches) return;
  state.watches = [];
  const concepts = self.concepts();
  for (const fields of [concepts.opened, concepts.held, concepts.branch]) {
    const consumer = document.createElement('span');
    consumer.hidden = true;
    const frame = () => self.refresh();
    consumer.reset = frame;
    consumer.update = frame;
    consumer.error = (payload) => console.warn('<tree-inspector> subscription:', payload);
    state.shadow.append(consumer);
    const watch = { consumer, cancel: null };
    state.watches.push(watch);
    const terms = { this: { '?': { name: 'this' } } };
    const descriptor = {};
    for (const [name, [the, as, more]] of Object.entries(fields)) {
      descriptor[name] = { the, as, cardinality: 'one', ...(more ?? {}) };
      terms[name] = { '?': { name } };
    }
    const query = { predicate: { with: descriptor }, terms };
    const subscribe = (attempt) => {
      if (!state.watches?.includes(watch) || !consumer.isConnected) return;
      const detail = { query, tag: 'tree' };
      const event = new CustomEvent('tonk-subscribe', { detail, bubbles: true, composed: true, cancelable: true });
      consumer.dispatchEvent(event);
      const cancel = detail.subscription?.cancel;
      if (event.defaultPrevented && typeof cancel === 'function') {
        watch.cancel = () => cancel.call(detail.subscription);
      } else if (attempt < 12) {
        setTimeout(() => subscribe(attempt + 1), 250 * Math.min(attempt + 1, 4));
      }
    };
    subscribe(0);
  }
}
