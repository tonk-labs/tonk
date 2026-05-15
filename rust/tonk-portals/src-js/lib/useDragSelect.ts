import { useEffect, useRef, useState, type RefObject } from "react";
import type { PixelRect } from "./grid";

type Args = {
  containerRef: RefObject<HTMLDivElement | null>;
  enabled: boolean;
  onCommit: (rect: PixelRect) => void;
  onCancel?: () => void;
};

export function useDragSelect({ containerRef, enabled, onCommit, onCancel }: Args) {
  const [draft, setDraft] = useState<PixelRect | null>(null);
  const startRef = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !enabled) return;

    const localCoords = (e: MouseEvent) => {
      const rect = container.getBoundingClientRect();
      return {
        x: Math.max(0, e.clientX - rect.left),
        y: Math.max(0, e.clientY - rect.top),
      };
    };

    const onMouseDown = (e: MouseEvent) => {
      if (e.target !== container) return;
      if (e.button !== 0) return;
      const { x, y } = localCoords(e);
      startRef.current = { x, y };
      setDraft({ x, y, w: 0, h: 0 });
      e.preventDefault();
    };

    const onMouseMove = (e: MouseEvent) => {
      const start = startRef.current;
      if (!start) return;
      const { x, y } = localCoords(e);
      const left = Math.min(start.x, x);
      const top = Math.min(start.y, y);
      const right = Math.max(start.x, x);
      const bottom = Math.max(start.y, y);
      setDraft({ x: left, y: top, w: right - left, h: bottom - top });
    };

    const onMouseUp = () => {
      const start = startRef.current;
      startRef.current = null;
      if (!start) return;
      setDraft((current) => {
        if (current && current.w > 4 && current.h > 4) {
          onCommit(current);
        } else {
          onCancel?.();
        }
        return null;
      });
    };

    container.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      container.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [containerRef, enabled, onCommit, onCancel]);

  return draft;
}
