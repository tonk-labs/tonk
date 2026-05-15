import { useCallback, useState } from "react";
import { WIDGET_SPAN, clamp, spanForWidgets } from "./grid";

export type Corner = "tl" | "tr" | "bl" | "br";
export type Edge = "t" | "r" | "b" | "l";
export type Handle = Corner | Edge;

type Args = {
  cellSize: number;
  startW: number;
  startH: number;
  startCol: number;
  startRow: number;
  maxW: number;
  maxH: number;
  onCommit: (next: { w: number; h: number; col: number; row: number }) => void;
};

export type ResizeGhost = { w: number; h: number; col: number; row: number };

const DIRS: Record<Handle, { x: number; y: number }> = {
  tl: { x: -1, y: -1 },
  tr: { x: 1, y: -1 },
  bl: { x: -1, y: 1 },
  br: { x: 1, y: 1 },
  t: { x: 0, y: -1 },
  r: { x: 1, y: 0 },
  b: { x: 0, y: 1 },
  l: { x: -1, y: 0 },
};

function placement(
  handle: Handle,
  startCol: number,
  startRow: number,
  startW: number,
  startH: number,
  newW: number,
  newH: number,
): { col: number; row: number } {
  const dW = spanForWidgets(startW) - spanForWidgets(newW);
  const dH = spanForWidgets(startH) - spanForWidgets(newH);
  const fromLeft = handle.includes("l");
  const fromTop = handle.includes("t");
  return {
    col: startCol + (fromLeft ? dW : 0),
    row: startRow + (fromTop ? dH : 0),
  };
}

export function useResize({
  cellSize,
  startW,
  startH,
  startCol,
  startRow,
  maxW,
  maxH,
  onCommit,
}: Args) {
  const [ghost, setGhost] = useState<ResizeGhost | null>(null);

  const beginResize = useCallback(
    (handle: Handle) => (e: React.PointerEvent) => {
      e.stopPropagation();
      e.preventDefault();
      const widgetSize = WIDGET_SPAN * cellSize;
      const dir = DIRS[handle];
      const startX = e.clientX;
      const startY = e.clientY;
      setGhost({ w: startW, h: startH, col: startCol, row: startRow });

      const onMove = (ev: PointerEvent) => {
        const dx = (ev.clientX - startX) * dir.x;
        const dy = (ev.clientY - startY) * dir.y;
        const startSizeW = startW * widgetSize;
        const startSizeH = startH * widgetSize;
        const rawW = Math.max(widgetSize, startSizeW + dx);
        const rawH = Math.max(widgetSize, startSizeH + dy);
        const w = clamp(Math.round(rawW / widgetSize), 1, maxW);
        const h = clamp(Math.round(rawH / widgetSize), 1, maxH);
        const { col, row } = placement(handle, startCol, startRow, startW, startH, w, h);
        setGhost({ w, h, col, row });
      };

      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        setGhost((g) => {
          if (g && (g.w !== startW || g.h !== startH)) {
            onCommit({ w: g.w, h: g.h, col: g.col, row: g.row });
          }
          return null;
        });
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [cellSize, startW, startH, startCol, startRow, maxW, maxH, onCommit],
  );

  return { ghost, beginResize };
}
