(self) => {
  const state = self.__tree;
  if (!state) return;
  state.observer?.disconnect();
  state.observer = null;
  if (state.frame) cancelAnimationFrame(state.frame);
  state.frame = 0;
}
