// Teardown waits a macrotask, and only happens if the notebook is still
// out of the document: a move re-attaches it within the task, and has to
// find its editor and listeners as they were. A notebook torn down mounts
// afresh when it is attached again.
(self) => {
  const state = self.__notebook;
  if (!state) return;
  state.detached = true;
  setTimeout(() => {
    if (!state.detached || self.isConnected || self.__notebook !== state) return;
    self.__notebook = null;
    state.observer?.disconnect();
    for (const timer of state.timers) clearTimeout(timer);
    state.timers.clear();
    document.removeEventListener('selectionchange', state.onSelection);
    state.prose?.shadowRoot?.removeEventListener('selectionchange', state.onSelection);
    state.prose?.remove();
    state.notice?.remove();
    state.watch?.cancel();
  }, 0);
}
