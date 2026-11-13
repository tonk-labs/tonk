// A route that arrives or moves after connect (a template stamping its
// `with`) starts over on the branch it now names, rather than staying on
// the "nothing to draw" it started with. `start` does nothing for the
// route it already shows, so the replay before `connected` costs nothing.
(self, name, before, after) => {
  if (before === after || !self.isConnected) return;
  if (name === 'with' || name === 'repo' || name === 'branch') self.start();
}
