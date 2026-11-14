// A `with` that arrives or moves after connect (a template stamping its
// placeholder) remounts on the branch it now names, rather than leaving
// the inspector on the branch, or the error, it started with.
(self, name) => {
  if (name !== 'with' || !self.isConnected) return;
  const context = self.context();
  if (context && self.__inspector?.context?.with === context.with) return;
  self.mount(context);
}
