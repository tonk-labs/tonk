import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { clamp, computeGridDims, spanForWidgets } from "../lib/presets";
import { pixelRectToCell } from "../lib/snap";
import { collidesWithAny, pack, packAroundBounded } from "../lib/pack";
import { useDragSelect } from "../lib/useDragSelect";
import { resolveName } from "../lib/resolveName";
import { isTileMessage, SQUARE_ID_ATTR, type TileMessage } from "../lib/tile-messages";
import { RepoContext } from "../context";
import type { Square as SquareT } from "../types";
import type { PickPayload } from "./EntityPicker";
import { Square } from "./Square";
import { SelectionRect } from "./SelectionRect";
import { GridOverlay } from "./GridOverlay";
import { Rail } from "./Rail";

let nextId = 1;
const makeId = () => `sq-${nextId++}`;

// Measure the grid container itself, not the window. The portals
// element lives inside the Leptos shell next to a banner and a
// sidebar, so the viewport is *not* the right reference.
function useElementSize(ref: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ w: 0, h: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const rect = el.getBoundingClientRect();
      setSize({ w: rect.width, h: rect.height });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}

export function Grid() {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const vp = useElementSize(wrapperRef);
  const [squares, setSquares] = useState<SquareT[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pickerForId, setPickerForId] = useState<string | null>(null);
  const repo = useContext(RepoContext);

  const { cols, rows, maxW, maxH, cellSize } = useMemo(
    () => computeGridDims(vp.w, vp.h),
    [vp.w, vp.h],
  );
  const gridWidth = cellSize * cols;
  const gridHeight = cellSize * rows;

  const active = useMemo(() => squares.filter((s) => !s.minimized), [squares]);
  const minimized = useMemo(() => squares.filter((s) => s.minimized), [squares]);

  const handleCommit = useCallback(
    (rect: { x: number; y: number; w: number; h: number }) => {
      if (cellSize <= 0) return;
      const snapped = pixelRectToCell(rect, cellSize, maxW, maxH);
      const w = snapped.w;
      const h = snapped.h;
      const sw = spanForWidgets(w);
      const sh = spanForWidgets(h);
      const col = clamp(snapped.col, 0, cols - sw);
      const row = clamp(snapped.row, 0, rows - sh);

      const newId = makeId();
      setSquares((prev) => {
        const activePrev = prev.filter((s) => !s.minimized);
        const minimizedPart = prev.filter((s) => s.minimized);
        const newSq: SquareT = {
          id: newId,
          w,
          h,
          col,
          row,
          minimized: false,
        };
        if (!collidesWithAny(activePrev, col, row, sw, sh, cols)) {
          return [...prev, newSq];
        }
        const { placed, overflow } = packAroundBounded(newSq, activePrev, cols, rows);
        const overflowIds = new Set(overflow.map((s) => s.id));
        const placedById = new Map(placed.map((s) => [s.id, s]));
        const repackedActive = activePrev.map((sq) => {
          if (overflowIds.has(sq.id)) return { ...sq, minimized: true };
          return placedById.get(sq.id) ?? sq;
        });
        return [...minimizedPart, ...repackedActive, newSq];
      });
      setSelectedId(newId);
      setPickerForId(newId);
    },
    [cellSize, cols, rows, maxW, maxH],
  );

  const handleOpenPicker = useCallback((id: string) => {
    setPickerForId(id);
  }, []);

  const handleClosePicker = useCallback(() => {
    setPickerForId(null);
  }, []);

  const handlePick = useCallback(
    (payload: PickPayload) => {
      const id = pickerForId;
      if (!id) return;
      setSquares((prev) =>
        prev.map((s) =>
          s.id === id
            ? {
                ...s,
                entity: payload.entity,
                name: payload.name,
                branch: payload.branch,
              }
            : s,
        ),
      );
      setPickerForId(null);
    },
    [pickerForId],
  );

  const draft = useDragSelect({
    containerRef,
    enabled: cellSize > 0,
    onCommit: handleCommit,
    onCancel: () => setSelectedId(null),
  });

  const handleSelect = useCallback((id: string) => {
    setSelectedId(id);
  }, []);

  const handleMove = useCallback(
    (id: string, next: { col: number; row: number }) => {
      setSquares((prev) => {
        const moved = prev.find((s) => s.id === id);
        if (!moved) return prev;
        const sw = spanForWidgets(moved.w);
        const sh = spanForWidgets(moved.h);
        const others = prev.filter((s) => s.id !== id && !s.minimized);
        const updatedMoved = { ...moved, col: next.col, row: next.row };
        if (!collidesWithAny(others, next.col, next.row, sw, sh, cols)) {
          return prev.map((s) => (s.id === id ? updatedMoved : s));
        }
        const { placed, overflow } = packAroundBounded(updatedMoved, others, cols, rows);
        const overflowIds = new Set(overflow.map((s) => s.id));
        const placedById = new Map(placed.map((s) => [s.id, s]));
        return prev.map((sq) => {
          if (sq.id === id) return updatedMoved;
          if (overflowIds.has(sq.id)) return { ...sq, minimized: true };
          return placedById.get(sq.id) ?? sq;
        });
      });
    },
    [cols, rows],
  );

  const handleResize = useCallback(
    (id: string, next: { w: number; h: number; col: number; row: number }) => {
      setSquares((prev) => {
        const sw = spanForWidgets(next.w);
        const sh = spanForWidgets(next.h);
        const col = clamp(next.col, 0, cols - sw);
        const row = clamp(next.row, 0, rows - sh);
        const updated = prev.map((sq) =>
          sq.id === id ? { ...sq, w: next.w, h: next.h, col, row } : sq,
        );
        const me = updated.find((s) => s.id === id);
        if (!me) return updated;
        const others = updated.filter((s) => s.id !== id && !s.minimized);
        if (!collidesWithAny(others, me.col, me.row, sw, sh, cols)) {
          return updated;
        }
        const { placed, overflow } = packAroundBounded(me, others, cols, rows);
        const overflowIds = new Set(overflow.map((s) => s.id));
        const placedById = new Map(placed.map((s) => [s.id, s]));
        return updated.map((sq) => {
          if (overflowIds.has(sq.id)) return { ...sq, minimized: true };
          return placedById.get(sq.id) ?? sq;
        });
      });
    },
    [cols, rows],
  );

  const handleFullscreen = useCallback(
    (id: string) => {
      setSquares((prev) =>
        prev.map((sq) => {
          if (sq.id === id) {
            return { ...sq, w: maxW, h: maxH, col: 0, row: 0, minimized: false };
          }
          if (!sq.minimized) return { ...sq, minimized: true };
          return sq;
        }),
      );
      setSelectedId(id);
    },
    [maxW, maxH],
  );

  const handleMinimize = useCallback((id: string) => {
    setSquares((prev) => prev.map((s) => (s.id === id ? { ...s, minimized: true } : s)));
    setSelectedId((cur) => (cur === id ? null : cur));
  }, []);

  const handleClose = useCallback(
    (id: string) => {
      setSquares((prev) => {
        const filtered = prev.filter((sq) => sq.id !== id);
        const activePart = filtered.filter((s) => !s.minimized);
        const minPart = filtered.filter((s) => s.minimized);
        return [...minPart, ...pack(activePart, cols)];
      });
      setSelectedId((cur) => (cur === id ? null : cur));
      setPickerForId((cur) => (cur === id ? null : cur));
    },
    [cols],
  );

  // Listen for postMessages from artifact iframes. The wire
  // contract lives in `lib/tile-messages.ts`; here we resolve the
  // posting iframe back to its owning square via the
  // `data-square-id` marker the wrapper carries, and dispatch on
  // `type`. Origin is checked against the shell's own origin —
  // `allow-same-origin` sandboxed iframes post under that origin,
  // so the equality holds.
  useEffect(() => {
    if (!repo) return;

    const findSquareIdFor = (source: MessageEventSource | null): string | null => {
      if (!source) return null;
      const iframes = document.querySelectorAll<HTMLIFrameElement>("iframe.square__iframe");
      for (const frame of iframes) {
        if (frame.contentWindow !== source) continue;
        const wrapper = frame.closest<HTMLElement>(`[${SQUARE_ID_ATTR}]`);
        return wrapper?.getAttribute(SQUARE_ID_ATTR) ?? null;
      }
      return null;
    };

    const handleNavigate = async (
      squareId: string,
      msg: Extract<TileMessage, { type: "tonk:navigate" }>,
    ) => {
      const targetBranch = msg.branch?.trim() || undefined;
      let entity = msg.entity?.trim();
      let name = msg.name?.trim() || undefined;

      if (!entity && name) {
        try {
          entity = await resolveName(repo, targetBranch ?? "main", name);
        } catch (err) {
          console.warn(`[tonk-portals] tonk:navigate resolve failed: ${err}`);
          return;
        }
      }

      if (!entity) {
        console.warn("[tonk-portals] tonk:navigate ignored: missing entity and name");
        return;
      }

      setSquares((prev) =>
        prev.map((s) =>
          s.id === squareId
            ? {
                ...s,
                entity,
                name: name ?? (entity === s.entity ? s.name : undefined),
                branch: targetBranch ?? s.branch,
              }
            : s,
        ),
      );
    };

    const onMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if (!isTileMessage(event.data)) return;
      const squareId = findSquareIdFor(event.source);
      if (!squareId) return;

      const msg = event.data;
      switch (msg.type) {
        case "tonk:navigate":
          void handleNavigate(squareId, msg);
          break;
        case "tonk:close":
          handleClose(squareId);
          break;
        default: {
          // Exhaustiveness — adding a new variant to TileMessage
          // without a case here makes TS error.
          const _exhaustive: never = msg;
          void _exhaustive;
        }
      }
    };

    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [repo, handleClose]);

  const handleRestore = useCallback(
    (id: string) => {
      setSquares((prev) => {
        const target = prev.find((s) => s.id === id);
        if (!target) return prev;
        const sw = spanForWidgets(target.w);
        const sh = spanForWidgets(target.h);
        const others = prev.filter((s) => s.id !== id && !s.minimized);
        const restored = { ...target, minimized: false };
        if (!collidesWithAny(others, restored.col, restored.row, sw, sh, cols)) {
          return prev.map((s) => (s.id === id ? restored : s));
        }
        const { placed, overflow } = packAroundBounded(restored, others, cols, rows);
        const overflowIds = new Set(overflow.map((s) => s.id));
        const placedById = new Map(placed.map((s) => [s.id, s]));
        return prev.map((sq) => {
          if (sq.id === id) return restored;
          if (overflowIds.has(sq.id)) return { ...sq, minimized: true };
          return placedById.get(sq.id) ?? sq;
        });
      });
    },
    [cols, rows],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!selectedId) return;
      if (e.key === "Backspace" || e.key === "Delete") {
        const target = e.target as HTMLElement | null;
        if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
        setSquares((prev) => {
          const filtered = prev.filter((sq) => sq.id !== selectedId);
          const activePart = filtered.filter((s) => !s.minimized);
          const minPart = filtered.filter((s) => s.minimized);
          return [...minPart, ...pack(activePart, cols)];
        });
        setSelectedId(null);
      } else if (e.key === "Escape") {
        setSelectedId(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedId, cols]);

  return (
    <div ref={wrapperRef} className="grid-wrapper">
      <div
        ref={containerRef}
        className={`grid${draft ? " grid--dragging" : ""}`}
        style={{ width: gridWidth, height: gridHeight }}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) setSelectedId(null);
        }}
      >
        <GridOverlay cellSize={cellSize} cols={cols} rows={rows} />
        {active.map((sq) => (
          <Square
            key={sq.id}
            square={sq}
            cellSize={cellSize}
            maxW={maxW}
            maxH={maxH}
            selected={sq.id === selectedId}
            pickerOpen={sq.id === pickerForId}
            onSelect={handleSelect}
            onResize={handleResize}
            onMove={handleMove}
            onMinimize={handleMinimize}
            onFullscreen={handleFullscreen}
            onClose={handleClose}
            onOpenPicker={handleOpenPicker}
            onPick={handlePick}
            onClosePicker={handleClosePicker}
          />
        ))}
        {draft && <SelectionRect rect={draft} cellSize={cellSize} maxW={maxW} maxH={maxH} />}
        {active.length === 0 && !draft && (
          <div className="grid__empty">Drag anywhere to create an artifact</div>
        )}
      </div>
      <Rail squares={minimized} onRestore={handleRestore} />
    </div>
  );
}
