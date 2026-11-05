// Dispatch the pending `change`, stamped with a fresh version. The store
// writes that envelope back, and its version being no newer is how the
// element knows the write is its own.
(self) => {
  const state = self.__prose;
  state.timer = null;
  const value = state.pending;
  state.pending = null;
  if (value === null) return;
  const hlc = self.tick();
  state.known = hlc;
  self.emit('change', { value, content: self.encode(hlc, value) });
}
