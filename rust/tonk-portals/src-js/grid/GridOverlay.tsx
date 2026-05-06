export function GridOverlay({
  cellSize,
  cols,
  rows,
}: {
  cellSize: number;
  cols: number;
  rows: number;
}) {
  if (cellSize <= 0) return null;
  const dots: { x: number; y: number }[] = [];
  for (let r = 0; r <= rows; r++) {
    for (let c = 0; c <= cols; c++) {
      dots.push({ x: c * cellSize, y: r * cellSize });
    }
  }
  return (
    <div className="grid-overlay">
      {dots.map((d, i) => (
        <div
          key={i}
          className="grid-dot"
          style={{ left: d.x, top: d.y }}
        />
      ))}
    </div>
  );
}
