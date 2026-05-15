// Grid layout primitives: dimensions, snap, pack, collision.
// Replaces the BSP tree from layout.ts with a flat cell-grid model.

export const PADDING = 8;
export const WIDGET_SPAN = 6;
const TARGET_WIDGET_PIXEL = 144;

export type GridDims = {
  cols: number;
  rows: number;
  maxW: number;
  maxH: number;
  cellSize: number;
};

export type PixelRect = { x: number; y: number; w: number; h: number };

export type DocWidth = "full" | "half";

export type Tile = {
  id: string;
  w: number;
  h: number;
  col: number;
  row: number;
  minimized: boolean;
  entity?: string;
  branch?: string;
  name?: string;
  docWidth?: DocWidth;
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

export function pixelRectToCell(
  rect: PixelRect,
  cellSize: number,
  maxW: number,
  maxH: number,
): { col: number; row: number; w: number; h: number } {
  const widgetSize = WIDGET_SPAN * cellSize;
  const wColStart = Math.floor(rect.x / widgetSize);
  const wRowStart = Math.floor(rect.y / widgetSize);
  const wColEnd = Math.ceil((rect.x + rect.w) / widgetSize);
  const wRowEnd = Math.ceil((rect.y + rect.h) / widgetSize);
  const w = clamp(Math.max(1, wColEnd - wColStart), 1, maxW);
  const h = clamp(Math.max(1, wRowEnd - wRowStart), 1, maxH);
  const centerX = rect.x + rect.w / 2;
  const centerY = rect.y + rect.h / 2;
  const tlX = centerX - (w * widgetSize) / 2;
  const tlY = centerY - (h * widgetSize) / 2;
  const widgetCol = clamp(Math.round(tlX / widgetSize), 0, maxW - w);
  const widgetRow = clamp(Math.round(tlY / widgetSize), 0, maxH - h);
  return {
    col: widgetCol * WIDGET_SPAN,
    row: widgetRow * WIDGET_SPAN,
    w,
    h,
  };
}

export function collidesWithAny(
  tiles: Tile[],
  col: number,
  row: number,
  sw: number,
  sh: number,
  cols: number,
): boolean {
  if (col < 0 || row < 0 || col + sw > cols) return true;
  for (const t of tiles) {
    const tw = spanForWidgets(t.w);
    const th = spanForWidgets(t.h);
    const overlapsX = col < t.col + tw && col + sw > t.col;
    const overlapsY = row < t.row + th && row + sh > t.row;
    if (overlapsX && overlapsY) return true;
  }
  return false;
}

export function pack(tiles: Tile[], cols: number): Tile[] {
  const occupied: boolean[][] = [];
  const isFree = (col: number, row: number, sw: number, sh: number): boolean => {
    if (col + sw > cols) return false;
    for (let r = row; r < row + sh; r++) {
      for (let c = col; c < col + sw; c++) {
        if (occupied[r]?.[c]) return false;
      }
    }
    return true;
  };
  const occupy = (col: number, row: number, sw: number, sh: number) => {
    for (let r = row; r < row + sh; r++) {
      if (!occupied[r]) occupied[r] = [];
      for (let c = col; c < col + sw; c++) {
        occupied[r]![c] = true;
      }
    }
  };
  return tiles.map((t) => {
    const sw = spanForWidgets(t.w);
    const sh = spanForWidgets(t.h);
    let row = 0;
    while (true) {
      for (let col = 0; col <= cols - sw; col++) {
        if (isFree(col, row, sw, sh)) {
          occupy(col, row, sw, sh);
          return { ...t, col, row };
        }
      }
      row++;
    }
  });
}

export function packAroundBounded(
  anchor: Tile,
  others: Tile[],
  cols: number,
  maxRows: number,
): { placed: Tile[]; overflow: Tile[] } {
  const occupied: boolean[][] = [];
  const occupy = (col: number, row: number, sw: number, sh: number) => {
    for (let r = row; r < row + sh; r++) {
      if (!occupied[r]) occupied[r] = [];
      for (let c = col; c < col + sw; c++) {
        occupied[r]![c] = true;
      }
    }
  };
  const isFree = (col: number, row: number, sw: number, sh: number): boolean => {
    if (col + sw > cols) return false;
    for (let r = row; r < row + sh; r++) {
      for (let c = col; c < col + sw; c++) {
        if (occupied[r]?.[c]) return false;
      }
    }
    return true;
  };
  occupy(anchor.col, anchor.row, spanForWidgets(anchor.w), spanForWidgets(anchor.h));
  const placed: Tile[] = [];
  const overflow: Tile[] = [];
  for (const t of others) {
    const sw = spanForWidgets(t.w);
    const sh = spanForWidgets(t.h);
    let found: { col: number; row: number } | null = null;
    for (let row = 0; row + sh <= maxRows && !found; row++) {
      for (let col = 0; col <= cols - sw; col++) {
        if (isFree(col, row, sw, sh)) {
          found = { col, row };
          break;
        }
      }
    }
    if (found) {
      occupy(found.col, found.row, sw, sh);
      placed.push({ ...t, col: found.col, row: found.row });
    } else {
      overflow.push(t);
    }
  }
  return { placed, overflow };
}
