(self) => {
  const state = self.__tree;
  if (!state || state.frame) return;
  state.frame = requestAnimationFrame(() => {
    state.frame = 0;
    if (self.isConnected) self.render();
  });
}
