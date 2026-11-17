// The tree's keys: up and down move, right opens or steps in, left
// closes or steps out, Enter and Space act as a click, Home and End jump.
(self, event) => {
  const state = self.__tree;
  const row = event.target.closest?.('.row');
  if (!row) return;
  const rows = [...state.shell.querySelectorAll('.outline .row')];
  const at = rows.indexOf(row);
  const move = (next) => {
    if (!next) return;
    for (const other of rows) other.tabIndex = -1;
    next.tabIndex = 0;
    next.focus();
  };
  const level = (item) => Number(item.getAttribute('aria-level'));
  const key = event.key;
  if (key === 'ArrowDown') move(rows[at + 1]);
  else if (key === 'ArrowUp') move(rows[at - 1]);
  else if (key === 'Home') move(rows[0]);
  else if (key === 'End') move(rows[rows.length - 1]);
  else if (key === 'ArrowRight') {
    if (row.getAttribute('aria-expanded') === 'false') row.click();
    else if (row.getAttribute('aria-expanded') === 'true') move(rows[at + 1]);
  } else if (key === 'ArrowLeft') {
    if (row.getAttribute('aria-expanded') === 'true') {
      self.__tree.selected = row.dataset.node;
      row.querySelector('.dot')?.click();
    } else {
      move(rows.slice(0, at).reverse().find((other) => level(other) < level(row)));
    }
  } else if (key === 'Enter' || key === ' ') {
    row.click();
  } else {
    return;
  }
  event.preventDefault();
}
