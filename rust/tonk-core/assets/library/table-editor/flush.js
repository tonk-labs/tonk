// Dispatch the pending `change`, stamped with a fresh version. The store
// writes that envelope back, and its version being no newer is how the
// element knows the write is its own.
(self) => {
  const state = self.__table;
  state.timer = null;
  if (!state.dirty || !state.grid) return;
  state.dirty = false;
  const hlc = state.clock.tick();
  state.known = hlc;
  self.emit('change', { value: state.grid.toCsv(), content: state.core.envelope(state.grid, hlc) });
}
