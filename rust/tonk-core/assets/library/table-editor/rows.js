// The renderer updates claim rows in bursts, one per changed claim; one
// short tick folds a burst into one reconcile.
(self) => {
  const state = self.__table;
  if (state.rows !== null) return;
  state.rows = setTimeout(() => {
    state.rows = null;
    self.apply();
  }, 30);
}
