// Follow the branch's head: each commit moves `dialog.branch/tree`, and
// each frame of this subscription is only a signal to read the roots
// again. Dispatched from a consumer of its own, which carries the
// `reset`/`update` methods the host calls. A host still starting may not
// claim the event yet, so it is offered again a few times.
(self) => {
  const state = self.__tree;
  if (state.watch) return;
  const consumer = document.createElement('span');
  consumer.hidden = true;
  const frame = () => self.roots();
  consumer.reset = frame;
  consumer.update = frame;
  consumer.error = (payload) => console.warn('<tree-inspector> subscription:', payload);
  state.shadow.append(consumer);
  const watch = state.watch = { consumer, cancel: null };
  const query = {
    predicate: { with: {
      tree: { the: 'dialog.branch/tree', as: 'Entity', cardinality: 'one' },
    } },
    terms: { this: { '?': { name: 'this' } }, tree: { '?': { name: 'tree' } } },
  };
  const subscribe = (attempt) => {
    if (state.watch !== watch || !consumer.isConnected) return;
    const detail = { query, tag: 'tree' };
    const event = new CustomEvent('tonk-subscribe', {
      detail, bubbles: true, composed: true, cancelable: true,
    });
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
