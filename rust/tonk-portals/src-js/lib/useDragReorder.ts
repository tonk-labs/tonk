import { useCallback, useState } from "react";

type Args = {
  index: number;
  onReorder: (from: number, to: number) => void;
};

// Pointer-driven vertical reorder for doc-mode blocks.
// Walks sibling `.square--doc` rects of the parent container and
// picks a drop target by the cursor's Y vs each sibling's center.
// Half-width blocks share a row, so we also factor X when picking
// the target — the cursor's column decides whether we drop before
// or after a half-width neighbour.
export function useDragReorder({ index, onReorder }: Args) {
  const [dragging, setDragging] = useState(false);

  const beginReorder = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      e.stopPropagation();

      const startTarget = e.currentTarget as HTMLElement;
      const square = startTarget.closest<HTMLElement>(".square");
      const parent = square?.parentElement;
      if (!square || !parent) return;

      const siblings = Array.from(
        parent.querySelectorAll<HTMLElement>(":scope > .square"),
      );
      const rects = siblings.map((s) => s.getBoundingClientRect());
      setDragging(true);

      let targetIndex = index;

      const computeTarget = (x: number, y: number): number => {
        // Find closest sibling center; insertion index = that sibling's
        // index, or +1 if the cursor is past its center on the row axis
        // it actually occupies. For full-width rows, only Y matters.
        // For shared rows (two halves), we tie-break on X.
        let bestIdx = 0;
        let bestDist = Infinity;
        for (let i = 0; i < rects.length; i++) {
          const r = rects[i]!;
          const cx = r.left + r.width / 2;
          const cy = r.top + r.height / 2;
          const dx = x - cx;
          const dy = y - cy;
          const d = dx * dx + dy * dy;
          if (d < bestDist) {
            bestDist = d;
            bestIdx = i;
          }
        }
        const r = rects[bestIdx]!;
        const cy = r.top + r.height / 2;
        const cx = r.left + r.width / 2;
        // If above the closest, insert before; below or right-of-center
        // on same row, insert after.
        let insertAt = bestIdx;
        if (y > cy + 1 || (Math.abs(y - cy) < r.height / 2 && x > cx)) {
          insertAt = bestIdx + 1;
        }
        // Translate insertion index → final index after removal of self.
        if (insertAt > index) insertAt -= 1;
        if (insertAt < 0) insertAt = 0;
        if (insertAt > siblings.length - 1) insertAt = siblings.length - 1;
        return insertAt;
      };

      const onMove = (ev: PointerEvent) => {
        targetIndex = computeTarget(ev.clientX, ev.clientY);
      };

      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        setDragging(false);
        if (targetIndex !== index) onReorder(index, targetIndex);
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [index, onReorder],
  );

  return { dragging, beginReorder };
}
