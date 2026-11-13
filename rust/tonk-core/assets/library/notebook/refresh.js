// Project the rows and bind whatever fences appeared, marked as the
// notebook's own write so the observer does not answer it.
(self) => {
  const state = self.__notebook;
  if (!state?.prose || state.settling) return;
  state.settling = true;
  try {
    self.project();
    self.fences();
  } finally {
    state.settling = false;
  }
}
