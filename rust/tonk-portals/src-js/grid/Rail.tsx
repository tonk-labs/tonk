import { useCallback, useState } from "react";
import type { Tile } from "../lib/grid";
import { colorForArtifact } from "../lib/color";

type Props = {
  tiles: Tile[];
  vertical?: boolean;
  onRestore: (id: string) => void;
  onReorder: (from: number, to: number) => void;
};

function labelFor(tile: Tile): string {
  return tile.name ?? tile.entity ?? "Empty";
}

const DRAG_THRESHOLD = 4; // px — past this, treat as drag, not click

export function Rail({ tiles, vertical = false, onRestore, onReorder }: Props) {
  const [draggingId, setDraggingId] = useState<string | null>(null);

  const beginInteraction = useCallback(
    (e: React.PointerEvent, fromIndex: number, id: string) => {
      // Plain left-click only; let context menu / middle-click fall through.
      if (e.button !== 0) return;
      e.preventDefault();

      const startX = e.clientX;
      const startY = e.clientY;
      const button = e.currentTarget as HTMLElement;
      const container = button.parentElement;
      if (!container) return;

      const siblings = Array.from(
        container.querySelectorAll<HTMLElement>(":scope > .rail__tab"),
      );
      const rects = siblings.map((s) => s.getBoundingClientRect());

      let dragging = false;
      let targetIndex = fromIndex;

      const onMove = (ev: PointerEvent) => {
        const dx = ev.clientX - startX;
        const dy = ev.clientY - startY;
        if (!dragging && Math.hypot(dx, dy) > DRAG_THRESHOLD) {
          dragging = true;
          setDraggingId(id);
        }
        if (!dragging) return;
        const point = vertical ? ev.clientY : ev.clientX;
        let nearest = fromIndex;
        let bestDist = Infinity;
        for (let i = 0; i < rects.length; i++) {
          const r = rects[i]!;
          const center = vertical
            ? r.top + r.height / 2
            : r.left + r.width / 2;
          const d = Math.abs(point - center);
          if (d < bestDist) {
            bestDist = d;
            nearest = i;
          }
        }
        targetIndex = nearest;
      };

      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        if (dragging) {
          setDraggingId(null);
          if (targetIndex !== fromIndex) onReorder(fromIndex, targetIndex);
        } else {
          // No drag — treat as a click and restore the tile.
          onRestore(id);
        }
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [onRestore, onReorder, vertical],
  );

  if (tiles.length === 0) return null;
  return (
    <div className={`rail${vertical ? " rail--vertical" : ""}`}>
      {tiles.map((tile, idx) => {
        const label = labelFor(tile);
        const color = colorForArtifact(tile);
        const isDragging = draggingId === tile.id;
        return (
          <button
            key={tile.id}
            className={`rail__tab${isDragging ? " rail__tab--dragging" : ""}`}
            onPointerDown={(e) => beginInteraction(e, idx, tile.id)}
            title={label}
            style={color ? { background: color.full } : undefined}
          >
            <span className="rail__tab-label">{label}</span>
          </button>
        );
      })}
    </div>
  );
}
