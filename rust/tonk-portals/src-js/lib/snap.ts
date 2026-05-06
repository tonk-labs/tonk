import { WIDGET_SPAN, clamp } from "./presets";
import type { PixelRect } from "../types";

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
