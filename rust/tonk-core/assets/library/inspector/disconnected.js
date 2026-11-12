// Stop listening for code-editor; the cells stay, for a re-attach.
(self) => {
  const state = self.__inspector;
  if (!state?.watch) return;
  state.watch.cancel();
  state.watch = null;
}
