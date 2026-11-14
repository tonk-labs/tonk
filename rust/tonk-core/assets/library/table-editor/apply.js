// Reconcile the grid against the claim rows the light DOM holds now.
(self) => {
  const state = self.__table;
  const subject = self.getAttribute('subject');
  if (!state?.grid || subject === null) return;
  const { sheets, cells, columns, rowSizes } = state.core.readRows(self, subject);
  state.grid.applyRows(sheets, cells, columns, rowSizes);
}
