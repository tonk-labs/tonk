// Teardown waits a macrotask: renderers detach and re-attach nodes while
// they move them, and a re-attach in that window keeps the editor.
(self) => {
  const state = self.__prose;
  if (!state || state.teardown) return;
  state.teardown = true;
  setTimeout(() => {
    if (!state.teardown) return;
    state.teardown = false;
    if (self.isConnected) return;
    // The last keystrokes before removal still reach the store.
    if (state.timer !== null) {
      clearTimeout(state.timer);
      self.flush();
    }
    state.observer?.disconnect();
    state.observer = null;
    state.editor?.destroy();
    state.editor = null;
  }, 0);
}
