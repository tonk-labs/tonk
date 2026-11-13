// Teardown waits a macrotask: renderers detach and re-attach nodes while
// they move them, and a re-attach in that window keeps the grid.
(self) => {
  const state = self.__table;
  if (!state || state.teardown) return;
  state.teardown = true;
  setTimeout(() => {
    if (!state.teardown) return;
    state.teardown = false;
    if (self.isConnected) return;
    // The last edits before removal still reach the store.
    if (state.timer !== null) {
      clearTimeout(state.timer);
      self.flush();
    }
    clearTimeout(state.rows);
    state.rows = null;
    state.observer?.disconnect();
    state.observer = null;
    state.token++;
    state.grid?.destroy();
    state.grid = null;
  }, 0);
}
