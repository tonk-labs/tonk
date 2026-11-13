// Run `run` after `ms`, unless the notebook is torn down first.
(self, run, ms) => {
  const state = self.__notebook;
  if (!state) return;
  const timer = setTimeout(() => {
    state.timers.delete(timer);
    if (self.__notebook === state) run();
  }, ms);
  state.timers.add(timer);
}
