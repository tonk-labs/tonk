import { WIDGET_SPAN } from "../lib/grid";

// Dots at widget intersections — same lattice as the drag-preview
// cells, so the snap grid reads as one mesh.
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
  const widgetSize = cellSize * WIDGET_SPAN;
  const wCols = Math.floor(cols / WIDGET_SPAN);
  const wRows = Math.floor(rows / WIDGET_SPAN);
  const dots: { x: number; y: number }[] = [];
  for (let r = 0; r <= wRows; r++) {
    for (let c = 0; c <= wCols; c++) {
      dots.push({ x: c * widgetSize, y: r * widgetSize });
    }
  }
  return (
    <div className="grid-overlay">
      {dots.map((d, i) => (
        <div key={i} className="grid-dot" style={{ left: d.x, top: d.y }} />
      ))}
    </div>
  );
}
