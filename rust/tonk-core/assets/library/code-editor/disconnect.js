// Detach the language client. The editor stays: it goes back to plain
// editing, and the client belongs to whoever supplied it.
(self) => {
  const state = self.__code;
  if (!state) return;
  state.client = null;
  state.attachment = null;
  state.errors = 0;
  state.editor?.setLsp([]);
}
