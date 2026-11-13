// Show the notice while the space has no code-editor, which every cell is.
//
// A library component cannot install another, so the inspector cannot
// fetch its editor itself. It asks the space whether the name
// `code-editor` refers to anything (`id:code-editor`, which an install
// publishes) and keeps a subscription to that answer, so installing it
// clears the notice. The cells need no help: a <code-editor> rendered
// before its definition sits inert and upgrades when the definition lands.
(self) => {
  const state = self.__inspector;
  if (!state || state.watch) return;
  const show = (missing) => {
    if (self.__inspector === state) state.notice.hidden = !missing;
  };
  if (customElements.get('code-editor')) return show(false);
  customElements.whenDefined('code-editor').then(() => show(false));

  const query = {
    predicate: { with: { entity: { the: 'db.name/referent', as: 'Entity', cardinality: 'one' } } },
    terms: { this: 'id:code-editor', entity: { '?': { name: 'entity' } } },
  };
  // Dispatched from a consumer carrying this inspector's `with`: the host
  // routes by the dispatching element's own context.
  const consumer = document.createElement('span');
  consumer.hidden = true;
  consumer.setAttribute('with', state.context.with);
  const check = async () => {
    if (customElements.get('code-editor')) return show(false);
    const detail = { query };
    const event = new CustomEvent('tonk-query', { detail, bubbles: true, composed: true, cancelable: true });
    consumer.dispatchEvent(event);
    if (!event.defaultPrevented || !detail.result) return;
    try {
      const rows = await detail.result;
      show(!(Array.isArray(rows) && rows.length > 0) && !customElements.get('code-editor'));
    } catch (error) {
      console.warn('<space-inspector> could not ask for code-editor:', error);
    }
  };
  consumer.reset = check;
  consumer.update = check;
  consumer.error = (payload) => console.warn('<space-inspector> code-editor subscription:', payload);
  self.append(consumer);
  const watch = (state.watch = {
    cancel: () => {
      watch.cancelled = true;
      try { watch.unsubscribe?.(); } catch {}
      consumer.remove();
    },
  });
  // A host still starting may not claim the subscription yet: offer it
  // again a few times while the inspector stays where it is.
  const subscribe = (attempt) => {
    if (watch.cancelled || !consumer.isConnected) return;
    const detail = { query, tag: 'code-editor' };
    const event = new CustomEvent('tonk-subscribe', { detail, bubbles: true, composed: true, cancelable: true });
    consumer.dispatchEvent(event);
    const cancel = detail.subscription?.cancel;
    if (event.defaultPrevented && typeof cancel === 'function') {
      watch.unsubscribe = () => cancel.call(detail.subscription);
    } else if (attempt < 12) {
      setTimeout(() => subscribe(attempt + 1), 250 * Math.min(attempt + 1, 4));
    }
  };
  check();
  subscribe(0);
}
