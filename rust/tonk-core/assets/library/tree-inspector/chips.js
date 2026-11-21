// A key in full, as the pane shows it: each part a solid chip that wraps
// as text, the ordering as its letter, a part's kind in its title.
(self, parts) => {
  const box = document.createElement('div');
  box.className = 'chips';
  for (const piece of parts ?? []) {
    const chip = document.createElement('span');
    chip.className = `part part--${piece.kind}`;
    chip.textContent = piece.kind === 'index' ? (piece.text[0] ?? '?').toUpperCase() : piece.text || piece.kind;
    chip.title = `${piece.kind}: ${piece.text}`;
    box.append(chip);
  }
  return box;
}
