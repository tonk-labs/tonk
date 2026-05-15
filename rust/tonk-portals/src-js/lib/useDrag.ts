import { useCallback, useState } from "react";
import { WIDGET_SPAN, clamp } from "./grid";

type Args = {
  cellSize: number;
  startCol: number;
  startRow: number;
  w: number;
  h: number;
  maxW: number;
  maxH: number;
  onCommit: (next: { col: number; row: number }) => void;
};

export type DragGhost = { col: number; row: number };

export function useDrag({ cellSize, startCol, startRow, w, h, maxW, maxH, onCommit }: Args) {
  const [ghost, setGhost] = useState<DragGhost | null>(null);

  const beginDrag = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const widgetSize = WIDGET_SPAN * cellSize;
      const startX = e.clientX;
      const startY = e.clientY;
      const startWCol = Math.round(startCol / WIDGET_SPAN);
      const startWRow = Math.round(startRow / WIDGET_SPAN);
      setGhost({ col: startCol, row: startRow });

      const onMove = (ev: PointerEvent) => {
        const dx = ev.clientX - startX;
        const dy = ev.clientY - startY;
        const dWCol = Math.round(dx / widgetSize);
        const dWRow = Math.round(dy / widgetSize);
        const wCol = clamp(startWCol + dWCol, 0, maxW - w);
        const wRow = clamp(startWRow + dWRow, 0, maxH - h);
        setGhost({ col: wCol * WIDGET_SPAN, row: wRow * WIDGET_SPAN });
      };

      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        setGhost((g) => {
          if (g && (g.col !== startCol || g.row !== startRow)) {
            onCommit({ col: g.col, row: g.row });
          }
          return null;
        });
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [cellSize, startCol, startRow, w, h, maxW, maxH, onCommit],
  );

  return { ghost, beginDrag };
}
