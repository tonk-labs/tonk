(self, row, width, state) => {
  const SVG = 'http://www.w3.org/2000/svg';
  const LANE = 16;
  const HEIGHT = 36;
  const MIDDLE = HEIGHT / 2;
  const x = (lane) => LANE / 2 + lane * LANE + 4;
  const color = (lane) => `var(--vc-lane-${lane % 6})`;
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('class', 'vc-rails');
  svg.setAttribute('width', String(width * LANE + 8));
  svg.setAttribute('viewBox', `0 0 ${width * LANE + 8} ${HEIGHT}`);
  svg.setAttribute('aria-hidden', 'true');
  const curve = (x1, y1, x2, y2, stroke) => {
    const path = document.createElementNS(SVG, 'path');
    const bend = (y1 + y2) / 2;
    path.setAttribute(
      'd',
      x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}C${x1} ${bend} ${x2} ${bend} ${x2} ${y2}`,
    );
    path.setAttribute('stroke', stroke);
    svg.appendChild(path);
  };
  row.before.forEach((waiting, lane) => {
    if (waiting === null) return;
    if (waiting === row.revision.id) {
      curve(x(lane), 0, x(row.lane), MIDDLE, color(lane));
    } else {
      curve(x(lane), 0, x(lane), HEIGHT, color(lane));
    }
  });
  for (const target of row.outgoing) {
    curve(x(row.lane), MIDDLE, x(target), HEIGHT, color(target));
  }
  const node = document.createElementNS(SVG, 'circle');
  node.setAttribute('cx', String(x(row.lane)));
  node.setAttribute('cy', String(MIDDLE));
  node.setAttribute('r', state.head ? '6' : '5');
  node.setAttribute('stroke', state.head ? 'var(--vc-head)' : color(row.lane));
  node.setAttribute(
    'fill',
    state.selected ? color(row.lane) : state.head ? 'var(--vc-head)' : 'var(--vc-raised)',
  );
  svg.appendChild(node);
  return svg;
}
