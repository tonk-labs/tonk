(self, markdown) => {
  const lines = String(markdown).replace(/\r\n?/g, '\n').split('\n');
  const deck = [[[]]];
  let fence = null;
  lines.forEach((line, i) => {
    const marker = line.match(/^\s{0,3}(`{3,}|~{3,})/);
    if (fence) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && !line.trim().slice(marker[1].length).trim()) fence = null;
      deck.at(-1).at(-1).push(line);
      return;
    }
    if (marker) { fence = marker[1]; deck.at(-1).at(-1).push(line); return; }
    const open = i === 0 || lines[i - 1].trim() === '';
    if (open && /^\s{0,3}(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) { deck.push([[]]); return; }
    if (open && /^\s{0,3}\\?--\s*$/.test(line)) { deck.at(-1).push([]); return; }
    deck.at(-1).at(-1).push(line);
  });
  return deck
    .map((stack) => stack
      .map((slide) => {
        const at = slide.findIndex((line) => /^\s*notes?:\s*$/i.test(line));
        const body = (at === -1 ? slide : slide.slice(0, at)).join('\n').trim();
        const notes = at === -1 ? '' : slide.slice(at + 1).join('\n').trim();
        return { body, notes };
      })
      .filter((slide) => slide.body || slide.notes))
    .filter((stack) => stack.length);
}
