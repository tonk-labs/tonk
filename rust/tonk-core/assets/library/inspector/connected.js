// Mount for the branch `with` names. A re-attach on the same branch (a
// renderer moving the node) keeps the cells and their history.
(self) => {
  const context = self.context();
  const state = self.__inspector;
  if (state && context && state.context?.with === context.with) {
    self.require();
    return;
  }
  self.mount(context);
}
