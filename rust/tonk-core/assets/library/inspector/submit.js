// Commit the cell: run its buffer for real, show what it did, seal it,
// and open the next one. A failure stays on the cell, which can be
// corrected and submitted again.
async (self, cell) => {
  if (cell.sealed || cell.running) return;
  const body = cell.buffer;
  if (!body.trim()) return;
  cell.running = true;
  try {
    const response = await self.evaluate(body, true);
    cell.slot.innerHTML = self.result(null, response);
    cell.running = false;
    cell.sealed = true;
    cell.refresh();
    // For a sync control on the page: a commit landed.
    window.dispatchEvent(new CustomEvent('tonk:committed'));
    self.__inspector?.panel?.refresh();
    self.cell(true);
  } catch (error) {
    cell.slot.innerHTML = self.result(error?.message ?? String(error), null);
    cell.running = false;
  }
}
