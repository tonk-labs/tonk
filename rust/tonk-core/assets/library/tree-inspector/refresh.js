// Read every concept the page shows, once at a time however many signals
// arrive meanwhile, and draw again.
async (self) => {
  const state = self.__tree;
  if (state.reading) {
    state.again = true;
    return;
  }
  state.reading = true;
  try {
    do {
      state.again = false;
      const concepts = self.concepts();
      const names = Object.keys(concepts);
      const answers = await Promise.all(names.map((name) => self.query(concepts[name])));
      const rows = Object.fromEntries(names.map((name, at) => [name, answers[at]]));
      state.model = self.read(rows);
      state.error = null;
      self.schedule();
    } while (state.again);
  } catch (error) {
    state.error = String(error?.message ?? error);
    self.schedule();
  } finally {
    state.reading = false;
  }
}
