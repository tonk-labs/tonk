// A separator in the outline: its parts as coloured text, the ordering
// as one letter (`E`, `A`, `V`, `H`), long parts shortened. A part past
// the one that tells this separator from both its neighbours does not
// decide where a lookup goes, so it is dimmed.
(self, parts, neighbors) => {
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const box = element('span', 'key');
  if (!parts) {
    box.append(element('span', 'part part--pending', '…'));
    return box;
  }
  // The first part at which `other` differs from these parts.
  const differs = (other) => {
    if (!other) return -1;
    let at = 0;
    while (at < parts.length && at < other.length && parts[at].kind === other[at].kind && parts[at].text === other[at].text) at += 1;
    return at;
  };
  const pivot = Math.max(differs(neighbors?.previous), differs(neighbors?.next));
  const elide = (text) => {
    const chars = [...text];
    return chars.length <= 18 ? text : `${chars.slice(0, 12).join('')}…${chars.slice(-4).join('')}`;
  };
  parts.forEach((piece, at) => {
    const text = piece.kind === 'index' ? (piece.text[0] ?? '?').toUpperCase() : elide(piece.text || piece.kind);
    const span = element('span', `part part--${piece.kind}${pivot >= 0 && at > pivot ? ' dim' : ''}`, text);
    span.title = `${piece.kind}: ${piece.text}`;
    box.append(span);
  });
  return box;
}
