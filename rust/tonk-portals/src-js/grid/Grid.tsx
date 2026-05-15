import { Fragment, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  clamp,
  computeGridDims,
  pixelRectToCell,
  spanForWidgets,
  type Tile,
} from "../lib/grid";
import { useDragSelect } from "../lib/useDragSelect";
import { isTileMessage, SQUARE_ID_ATTR, type TileMessage } from "../lib/tile-messages";
import { resolveName } from "../lib/resolveName";
import { RepoContext, ViewModeContext } from "../context";
import type { PickPayload } from "./EntityPicker";
import { Square } from "./Square";
import { GridOverlay } from "./GridOverlay";
import { SelectionRect } from "./SelectionRect";
import { Rail } from "./Rail";

let nextId = 1;
const makeId = () => `tile-${nextId++}`;

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
  const repo = useContext(RepoContext);
  const viewMode = useContext(ViewModeContext);

  const [tiles, setTiles] = useState<Tile[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pickerForId, setPickerForId] = useState<string | null>(null);
  const [fullscreenId, setFullscreenId] = useState<string | null>(null);

  const { cols, rows, maxW, maxH, cellSize } = useMemo(
    () => computeGridDims(vp.w, vp.h),
    [vp.w, vp.h],
  );
  const gridWidth = cellSize * cols;
  const gridHeight = cellSize * rows;
  const gridLeft = Math.max(0, (vp.w - gridWidth) / 2);
  const gridTop = Math.max(0, (vp.h - gridHeight) / 2);

  const active = useMemo(() => tiles.filter((t) => !t.minimized), [tiles]);
  const minimized = useMemo(() => tiles.filter((t) => t.minimized), [tiles]);

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
      setTiles((prev) => [
        ...prev,
        { id: newId, w, h, col, row, minimized: false },
      ]);
      setSelectedId(newId);
      setPickerForId(newId);
    },
    [cellSize, cols, rows, maxW, maxH],
  );

  const handleSelect = useCallback((id: string) => setSelectedId(id), []);

  const handleMove = useCallback(
    (id: string, next: { col: number; row: number }) => {
      setTiles((prev) =>
        prev.map((t) =>
          t.id === id ? { ...t, col: next.col, row: next.row } : t,
        ),
      );
    },
    [],
  );

  const handleResize = useCallback(
    (id: string, next: { w: number; h: number; col: number; row: number }) => {
      setTiles((prev) => {
        const sw = spanForWidgets(next.w);
        const sh = spanForWidgets(next.h);
        const col = clamp(next.col, 0, cols - sw);
        const row = clamp(next.row, 0, rows - sh);
        return prev.map((t) =>
          t.id === id ? { ...t, w: next.w, h: next.h, col, row } : t,
        );
      });
    },
    [cols],
  );

  const handleFullscreen = useCallback((id: string) => {
    setFullscreenId((cur) => (cur === id ? null : id));
    setSelectedId(id);
  }, []);

  const handleMinimize = useCallback((id: string) => {
    setTiles((prev) => prev.map((t) => (t.id === id ? { ...t, minimized: true } : t)));
    setSelectedId((cur) => (cur === id ? null : cur));
    setPickerForId((cur) => (cur === id ? null : cur));
    setFullscreenId((cur) => (cur === id ? null : cur));
  }, []);

  const handleClose = useCallback((id: string) => {
    setTiles((prev) => prev.filter((t) => t.id !== id));
    setSelectedId((cur) => (cur === id ? null : cur));
    setPickerForId((cur) => (cur === id ? null : cur));
    setFullscreenId((cur) => (cur === id ? null : cur));
  }, []);

  const handleRestore = useCallback((id: string) => {
    setTiles((prev) =>
      prev.map((t) => (t.id === id ? { ...t, minimized: false } : t)),
    );
    setSelectedId(id);
  }, []);

  const handleReorder = useCallback((from: number, to: number) => {
    setTiles((prev) => {
      const activePart = prev.filter((t) => !t.minimized);
      const minPart = prev.filter((t) => t.minimized);
      if (from < 0 || from >= activePart.length) return prev;
      const clampedTo = Math.max(0, Math.min(activePart.length - 1, to));
      if (clampedTo === from) return prev;
      const next = [...activePart];
      const [moved] = next.splice(from, 1);
      next.splice(clampedTo, 0, moved!);
      return [...minPart, ...next];
    });
  }, []);

  const handleInsertAt = useCallback((activeIndex: number) => {
    const newId = makeId();
    setTiles((prev) => {
      const activePart = prev.filter((t) => !t.minimized);
      const minPart = prev.filter((t) => t.minimized);
      const insertIdx = Math.max(0, Math.min(activePart.length, activeIndex));
      const newTile: Tile = {
        id: newId,
        w: 2,
        h: 2,
        col: 0,
        row: 0,
        minimized: false,
      };
      const next = [...activePart];
      next.splice(insertIdx, 0, newTile);
      return [...minPart, ...next];
    });
    setSelectedId(newId);
    setPickerForId(newId);
  }, []);

  const handleRailReorder = useCallback((from: number, to: number) => {
    setTiles((prev) => {
      const minPart = prev.filter((t) => t.minimized);
      const activePart = prev.filter((t) => !t.minimized);
      if (from < 0 || from >= minPart.length) return prev;
      const clampedTo = Math.max(0, Math.min(minPart.length - 1, to));
      if (clampedTo === from) return prev;
      const next = [...minPart];
      const [moved] = next.splice(from, 1);
      next.splice(clampedTo, 0, moved!);
      return [...next, ...activePart];
    });
  }, []);

  const handleToggleDocWidth = useCallback((id: string) => {
    setTiles((prev) =>
      prev.map((t) =>
        t.id === id
          ? { ...t, docWidth: (t.docWidth ?? "full") === "full" ? "half" : "full" }
          : t,
      ),
    );
  }, []);

  const handleOpenPicker = useCallback((id: string) => setPickerForId(id), []);
  const handleClosePicker = useCallback(() => setPickerForId(null), []);

  const handlePick = useCallback(
    (payload: PickPayload) => {
      const id = pickerForId;
      if (!id) return;
      setTiles((prev) =>
        prev.map((t) =>
          t.id === id ? { ...t, entity: payload.entity, name: payload.name } : t,
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

  useEffect(() => {
    if (!repo) return;

    const findTileIdFor = (source: MessageEventSource | null): string | null => {
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
      tileId: string,
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
      setTiles((prev) =>
        prev.map((t) =>
          t.id === tileId ? { ...t, entity, name, branch: targetBranch } : t,
        ),
      );
    };

    const onMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if (!isTileMessage(event.data)) return;
      const tileId = findTileIdFor(event.source);
      if (!tileId) return;
      const msg = event.data;
      switch (msg.type) {
        case "tonk:navigate":
          void handleNavigate(tileId, msg);
          break;
        case "tonk:close":
          handleClose(tileId);
          break;
        default: {
          const _exhaustive: never = msg;
          void _exhaustive;
        }
      }
    };

    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [repo, handleClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && fullscreenId) {
        setFullscreenId(null);
        return;
      }
      if (!selectedId) return;
      if (e.key === "Backspace" || e.key === "Delete") {
        const target = e.target as HTMLElement | null;
        if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
        handleClose(selectedId);
      } else if (e.key === "Escape") {
        setSelectedId(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedId, fullscreenId, handleClose]);

  const fullscreenTile = fullscreenId != null ? active.find((t) => t.id === fullscreenId) ?? null : null;
  const inFullscreen = fullscreenTile != null;

  const gridStyle: React.CSSProperties = {
    position: "absolute",
    left: gridLeft,
    top: gridTop,
    width: gridWidth,
    height: gridHeight,
  };

  if (viewMode === "doc") {
    return (
      <div ref={wrapperRef} className="grid-wrapper grid-wrapper--doc">
        <div className="grid--doc">
          <DocInserter onInsert={() => handleInsertAt(0)} />
          {active.map((tile, idx) => (
            <Fragment key={tile.id}>
              <Square
                tile={tile}
                x={0}
                y={0}
                w={0}
                h={0}
                cellSize={cellSize}
                maxW={maxW}
                maxH={maxH}
                selected={tile.id === selectedId}
                pickerOpen={tile.id === pickerForId}
                fullscreen={false}
                mode="doc"
                index={idx}
                onSelect={handleSelect}
                onMove={handleMove}
                onResize={handleResize}
                onClose={handleClose}
                onMinimize={handleMinimize}
                onFullscreen={handleFullscreen}
                onOpenPicker={handleOpenPicker}
                onPick={handlePick}
                onClosePicker={handleClosePicker}
                onReorder={handleReorder}
                onToggleDocWidth={handleToggleDocWidth}
              />
              <DocInserter onInsert={() => handleInsertAt(idx + 1)} />
            </Fragment>
          ))}
        </div>
        <Rail
          tiles={minimized}
          vertical
          onRestore={handleRestore}
          onReorder={handleRailReorder}
        />
      </div>
    );
  }

  return (
    <div ref={wrapperRef} className="grid-wrapper">
      <div
        className="grid-stage"
        style={{ width: vp.w, height: vp.h }}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) setSelectedId(null);
        }}
      >
        <div
          ref={containerRef}
          className={`grid${draft ? " grid--dragging" : ""}${tiles.length === 0 ? " grid--empty" : ""}`}
          style={gridStyle}
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setSelectedId(null);
          }}
        >
          <GridOverlay cellSize={cellSize} cols={cols} rows={rows} />
          {!inFullscreen &&
            active.map((tile) => (
              <Square
                key={tile.id}
                tile={tile}
                x={tile.col * cellSize}
                y={tile.row * cellSize}
                w={spanForWidgets(tile.w) * cellSize}
                h={spanForWidgets(tile.h) * cellSize}
                cellSize={cellSize}
                maxW={maxW}
                maxH={maxH}
                selected={tile.id === selectedId}
                pickerOpen={tile.id === pickerForId}
                fullscreen={false}
                onSelect={handleSelect}
                onMove={handleMove}
                onResize={handleResize}
                onClose={handleClose}
                onMinimize={handleMinimize}
                onFullscreen={handleFullscreen}
                onOpenPicker={handleOpenPicker}
                onPick={handlePick}
                onClosePicker={handleClosePicker}
              />
            ))}
          {inFullscreen && fullscreenTile && (
            <Square
              key={fullscreenTile.id}
              tile={fullscreenTile}
              x={0}
              y={0}
              w={gridWidth}
              h={gridHeight}
              cellSize={cellSize}
              maxW={maxW}
              maxH={maxH}
              selected={true}
              pickerOpen={fullscreenTile.id === pickerForId}
              fullscreen={true}
              onSelect={handleSelect}
              onMove={handleMove}
              onResize={handleResize}
              onClose={handleClose}
              onMinimize={handleMinimize}
              onFullscreen={handleFullscreen}
              onOpenPicker={handleOpenPicker}
              onPick={handlePick}
              onClosePicker={handleClosePicker}
            />
          )}
          {!inFullscreen && draft && (
            <SelectionRect rect={draft} cellSize={cellSize} maxW={maxW} maxH={maxH} />
          )}
          {tiles.length === 0 && !draft && (
            <div className="grid__empty">Drag anywhere to create a tile</div>
          )}
        </div>
        <Rail
          tiles={minimized}
          onRestore={handleRestore}
          onReorder={handleRailReorder}
        />
      </div>
    </div>
  );
}

function DocInserter({ onInsert }: { onInsert: () => void }) {
  return (
    <div
      className="doc-inserter"
      onClick={(e) => {
        e.stopPropagation();
        onInsert();
      }}
      aria-label="add block"
    >
      <button
        type="button"
        className="doc-inserter__btn"
        onClick={(e) => {
          e.stopPropagation();
          onInsert();
        }}
        aria-label="add block"
      >
        +
      </button>
    </div>
  );
}
