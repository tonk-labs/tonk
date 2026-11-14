// Subscribe to the records for `word`, so the block redraws when one is
// installed, edited or removed. The frames are only a signal: each one
// re-reads, as the element registry does. Dispatched from a consumer of
// its own inside the shadow root, which carries the `reset`/`update`
// methods the host calls with each frame.
//
// A host still starting may not claim the event yet, so it is offered
// again a few times, as long as the element stays where it was.
(self, word) => {
  const state = self.__code;
  const consumer = document.createElement('span');
  consumer.hidden = true;
  const frame = () => { if (state.wanted === word) self.resolve(word); };
  consumer.reset = frame;
  consumer.update = frame;
  consumer.error = (payload) => console.warn(`<code-editor> language "${word}" subscription:`, payload);
  state.shadow.append(consumer);
  const watch = state.watch = { consumer, cancel: null };
  const subscribe = (attempt) => {
    if (state.watch !== watch || !consumer.isConnected) return;
    const detail = { query: self.request(word), tag: 'language' };
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
