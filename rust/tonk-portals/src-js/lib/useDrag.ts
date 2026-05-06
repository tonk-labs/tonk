import { useCallback, useState } from "react";
import { WIDGET_SPAN, clamp } from "./presets";

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

      // See useResize for the rationale: capture the pointer on
      // the bar so iframe boxes inside the grid don't hijack the
      // move events as the cursor drags across them.
      const target = e.currentTarget as HTMLElement;
      try {
        target.setPointerCapture(e.pointerId);
      } catch {
        // already released — listeners below still fire for in-flight
        // events.
      }

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
        target.removeEventListener("pointermove", onMove);
        target.removeEventListener("pointerup", onUp);
        target.removeEventListener("pointercancel", onUp);
        try {
          target.releasePointerCapture(e.pointerId);
        } catch {
          // already released — fine.
        }
        setGhost((g) => {
          if (g && (g.col !== startCol || g.row !== startRow)) {
            onCommit({ col: g.col, row: g.row });
          }
          return null;
        });
      };

      target.addEventListener("pointermove", onMove);
      target.addEventListener("pointerup", onUp);
      target.addEventListener("pointercancel", onUp);
    },
    [cellSize, startCol, startRow, w, h, maxW, maxH, onCommit],
  );

  return { ghost, beginDrag };
}
