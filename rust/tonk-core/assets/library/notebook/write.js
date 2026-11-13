// Commit `text`, a document of this notebook's commands.
//
// When it fails nothing of it landed: stop waiting for its inserts, and
// forget the base text, so the next projection reads the editor's text as
// typed and commits it again, after a pause, so a write that keeps failing
// does not spin.
(self, text) => {
  const state = self.__notebook;
  self.evaluate(text, true).catch((error) => {
    console.error('notebook: commit failed:', error?.message ?? error);
    if (self.__notebook !== state) return;
    state.pending.inserts = [];
    state.projected = '';
    self.later(() => self.project(), 2000);
  });
}
