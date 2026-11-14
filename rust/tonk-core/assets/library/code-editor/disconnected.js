// Teardown waits a macrotask: renderers detach and re-attach nodes while
// they move them, and a re-attach in that window keeps the editor.
(self) => {
  const state = self.__code;
  if (!state || state.teardown) return;
  state.teardown = true;
  setTimeout(() => {
    if (!state.teardown) return;
    state.teardown = false;
    if (self.isConnected) return;
    self.unwatch();
    self.withdraw();
    state.editor?.destroy();
    state.editor = null;
    state.source = null;
  }, 0);
}
