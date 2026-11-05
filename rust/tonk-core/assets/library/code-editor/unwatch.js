(self) => {
  const state = self.__code;
  const watch = state?.watch;
  if (!watch) return;
  state.watch = null;
  try { watch.cancel?.(); } catch {}
  watch.consumer.remove();
}
