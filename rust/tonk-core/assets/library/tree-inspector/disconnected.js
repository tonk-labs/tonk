(self) => {
  const state = self.__tree;
  if (!state) return;
  for (const watch of state.watches ?? []) {
    try { watch.cancel?.(); } catch {}
    watch.consumer.remove();
  }
  state.watches = null;
  if (state.frame) cancelAnimationFrame(state.frame);
  state.frame = 0;
}
