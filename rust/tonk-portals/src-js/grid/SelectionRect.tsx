import { WIDGET_SPAN } from "../lib/presets";
import { pixelRectToCell } from "../lib/snap";
import type { PixelRect } from "../types";

export function SelectionRect({
  rect,
  cellSize,
  maxW,
  maxH,
}: {
  rect: PixelRect;
  cellSize: number;
  maxW: number;
  maxH: number;
}) {
  if (cellSize <= 0) return null;
  const widgetSize = WIDGET_SPAN * cellSize;
  const { col, row, w, h } = pixelRectToCell(rect, cellSize, maxW, maxH);
  const x = col * cellSize;
  const y = row * cellSize;

  const cells: { c: number; r: number }[] = [];
  for (let r = 0; r < h; r++) {
    for (let c = 0; c < w; c++) {
      cells.push({ c, r });
    }
  }

  return (
    <>
      {cells.map(({ c, r }) => (
        <div
          key={`${r}-${c}`}
          className="selection-cell"
          style={{
            transform: `translate(${x + c * widgetSize}px, ${y + r * widgetSize}px)`,
            width: widgetSize,
            height: widgetSize,
          }}
        />
      ))}
    </>
  );
}
