(self) => {
  const state = self.__tree;
  if (!state) return;
  if (state.frame) cancelAnimationFrame(state.frame);
  state.frame = 0;
  const watch = state.watch;
  state.watch = null;
  try { watch?.cancel?.(); } catch {}
  watch?.consumer.remove();
}
