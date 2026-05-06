import { spanForWidgets } from "./presets";
import type { Square } from "../types";

export function pack(squares: Square[], cols: number): Square[] {
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

  return squares.map((sq) => {
    const sw = spanForWidgets(sq.w);
    const sh = spanForWidgets(sq.h);
    let row = 0;
    while (true) {
      for (let col = 0; col <= cols - sw; col++) {
        if (isFree(col, row, sw, sh)) {
          occupy(col, row, sw, sh);
          return { ...sq, col, row };
        }
      }
      row++;
    }
  });
}

export function packAroundBounded(
  anchor: Square,
  others: Square[],
  cols: number,
  maxRows: number,
): { placed: Square[]; overflow: Square[] } {
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

  const placed: Square[] = [];
  const overflow: Square[] = [];

  for (const sq of others) {
    const sw = spanForWidgets(sq.w);
    const sh = spanForWidgets(sq.h);
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
      placed.push({ ...sq, col: found.col, row: found.row });
    } else {
      overflow.push(sq);
    }
  }

  return { placed, overflow };
}

export function collidesWithAny(
  squares: Square[],
  col: number,
  row: number,
  sw: number,
  sh: number,
  cols: number,
  ignoreId?: string,
): boolean {
  if (col < 0 || row < 0 || col + sw > cols) return true;
  for (const sq of squares) {
    if (sq.id === ignoreId) continue;
    const sqW = spanForWidgets(sq.w);
    const sqH = spanForWidgets(sq.h);
    const overlapsX = col < sq.col + sqW && col + sw > sq.col;
    const overlapsY = row < sq.row + sqH && row + sh > sq.row;
    if (overlapsX && overlapsY) return true;
  }
  return false;
}
