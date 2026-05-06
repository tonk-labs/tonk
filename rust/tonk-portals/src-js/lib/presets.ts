export const PADDING = 56;
export const WIDGET_SPAN = 6;
export const TARGET_WIDGET_PIXEL = 144;

export type GridDims = {
  cols: number;
  rows: number;
  maxW: number;
  maxH: number;
  cellSize: number;
};

export function computeGridDims(vpW: number, vpH: number): GridDims {
  const availW = vpW - 2 * PADDING;
  const availH = vpH - 2 * PADDING;
  if (availW <= 0 || availH <= 0) {
    return { cols: 0, rows: 0, maxW: 0, maxH: 0, cellSize: 0 };
  }
  const maxW = Math.max(1, Math.round(availW / TARGET_WIDGET_PIXEL));
  const maxH = Math.max(1, Math.round(availH / TARGET_WIDGET_PIXEL));
  const cellSizeX = availW / (maxW * WIDGET_SPAN);
  const cellSizeY = availH / (maxH * WIDGET_SPAN);
  const cellSize = Math.min(cellSizeX, cellSizeY);
  return {
    cols: maxW * WIDGET_SPAN,
    rows: maxH * WIDGET_SPAN,
    maxW,
    maxH,
    cellSize,
  };
}

export function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n));
}

export function spanForWidgets(widgets: number): number {
  return widgets * WIDGET_SPAN;
}

export function labelFor(w: number, h: number): string {
  if (w === 1 && h === 1) return "widget";
  return `${w}×${h}`;
}
