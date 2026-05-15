import { useContext, useRef, useState } from "react";
import { EntityPicker, type PickPayload } from "./EntityPicker";
import { colorForArtifact } from "../lib/color";
import { SQUARE_ID_ATTR } from "../lib/tile-messages";
import { WIDGET_SPAN, type Tile } from "../lib/grid";
import { useDrag } from "../lib/useDrag";
import { useDragReorder } from "../lib/useDragReorder";
import { useResize, type Corner, type Edge } from "../lib/useResize";
import { HostContext, RepoContext } from "../context";

type Props = {
  tile: Tile;
  x: number;
  y: number;
  w: number;
  h: number;
  cellSize: number;
  maxW: number;
  maxH: number;
  selected: boolean;
  pickerOpen: boolean;
  fullscreen: boolean;
  onSelect: (id: string) => void;
  onMove: (id: string, next: { col: number; row: number }) => void;
  onResize: (id: string, next: { w: number; h: number; col: number; row: number }) => void;
  onClose: (id: string) => void;
  onMinimize: (id: string) => void;
  onFullscreen: (id: string) => void;
  onOpenPicker: (id: string) => void;
  onPick: (payload: PickPayload) => void;
  onClosePicker: () => void;
  mode?: "canvas" | "doc";
  index?: number;
  onReorder?: (from: number, to: number) => void;
  onToggleDocWidth?: (id: string) => void;
};

const CORNERS: Corner[] = ["tl", "tr", "bl", "br"];
const EDGES: Edge[] = ["t", "r", "b", "l"];

function tileSrc(repo: string, host: string, entity: string, branch: string): string {
  return `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(branch)}/host/${encodeURIComponent(host)}/${entity}`;
}

function IconLink() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <path d="M5.5 8.5a2.5 2.5 0 0 0 3.5 0l1-1a2.5 2.5 0 0 0-3.5-3.5l-.5.5" />
      <path d="M8.5 5.5a2.5 2.5 0 0 0-3.5 0l-1 1a2.5 2.5 0 0 0 3.5 3.5l.5-.5" />
    </svg>
  );
}

function IconRefresh() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M11.5 7a4.5 4.5 0 1 1-1.1-2.9" />
      <path d="M10.5 1.5v3h-3" />
    </svg>
  );
}

function IconCode() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 4.5 1 7l3 2.5" />
      <path d="M10 4.5 13 7l-3 2.5" />
      <path d="M8.5 2l-3 10" />
    </svg>
  );
}

function IconWidth({ half }: { half: boolean }) {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      {half ? (
        <>
          <rect x="1.5" y="4" width="4.5" height="6" rx="1" />
          <rect x="8" y="4" width="4.5" height="6" rx="1" />
        </>
      ) : (
        <rect x="1.5" y="4" width="11" height="6" rx="1" />
      )}
    </svg>
  );
}

function IconLock({ locked }: { locked: boolean }) {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <rect x="2.5" y="6.5" width="9" height="6.5" rx="1.5" />
      {locked ? (
        <path d="M4.5 6.5V4.5a2.5 2.5 0 0 1 5 0v2" />
      ) : (
        <path d="M4.5 6.5V4.5a2.5 2.5 0 0 1 5 0" />
      )}
    </svg>
  );
}

export function Square({
  tile,
  x,
  y,
  w,
  h,
  cellSize,
  maxW,
  maxH,
  selected,
  pickerOpen,
  fullscreen,
  onSelect,
  onMove,
  onResize,
  onClose,
  onMinimize,
  onFullscreen,
  onOpenPicker,
  onPick,
  onClosePicker,
  mode = "canvas",
  index = 0,
  onReorder,
  onToggleDocWidth,
}: Props) {
  const repo = useContext(RepoContext);
  const host = useContext(HostContext);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [locked, setLocked] = useState(false);

  const { ghost: resizeGhost, beginResize } = useResize({
    cellSize,
    startW: tile.w,
    startH: tile.h,
    startCol: tile.col,
    startRow: tile.row,
    maxW,
    maxH,
    onCommit: (next) => onResize(tile.id, next),
  });

  const { ghost: dragGhost, beginDrag } = useDrag({
    cellSize,
    startCol: tile.col,
    startRow: tile.row,
    w: tile.w,
    h: tile.h,
    maxW,
    maxH,
    onCommit: (next) => onMove(tile.id, next),
  });

  const { dragging: reorderActive, beginReorder } = useDragReorder({
    index,
    onReorder: (from, to) => onReorder?.(from, to),
  });

  const widgetSize = WIDGET_SPAN * cellSize;
  const ghost = resizeGhost
    ? { ...resizeGhost }
    : dragGhost
      ? { col: dragGhost.col, row: dragGhost.row, w: tile.w, h: tile.h }
      : null;
  const isActive = !!ghost;

  const color = colorForArtifact(tile);
  const branch = tile.branch ?? "main";
  const titleLabel = tile.name ?? tile.entity;
  const canRender = !!repo && !!host && !!tile.entity;
  const src = canRender ? tileSrc(repo!, host!, tile.entity!, branch) : null;

  const handleCopyLink = (e: React.MouseEvent) => {
    e.stopPropagation();
    const text = src ?? window.location.href;
    void navigator.clipboard.writeText(text);
  };

  const handleRefresh = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (iframeRef.current) {
      iframeRef.current.src = iframeRef.current.src;
    }
  };

  const handleViewSource = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (src) window.open(`view-source:${window.location.origin}${src}`, "_blank");
  };

  const handleLock = (e: React.MouseEvent) => {
    e.stopPropagation();
    setLocked((l) => !l);
  };

  const isDoc = mode === "doc";
  const docWidth = tile.docWidth ?? "full";
  const canDrag = !fullscreen && !locked && !isDoc;
  const canResize = !fullscreen && !locked && !isDoc;
  const canReorder = !fullscreen && !locked && isDoc;

  const handleToggleWidth = (e: React.MouseEvent) => {
    e.stopPropagation();
    onToggleDocWidth?.(tile.id);
  };

  const squareClass = [
    "square",
    selected ? "square--selected" : "",
    isActive ? "square--active" : "",
    isDoc ? "square--doc" : "",
    isDoc ? `square--doc-${docWidth}` : "",
    reorderActive ? "square--reorder-active" : "",
  ]
    .filter(Boolean)
    .join(" ");

  // Stable per-tile view-transition-name so the browser morphs each
  // tile between its canvas position/size and its doc grid slot when
  // the mode switches. Skipped during fullscreen (single tile, no
  // useful morph). The name must be a CSS ident — tile IDs already
  // are (`tile-1`, `tile-2`, …) so we just prefix them.
  const vtName = !fullscreen ? `tp-sq-${tile.id}` : undefined;
  const squareStyle: React.CSSProperties = isDoc
    ? { viewTransitionName: vtName }
    : {
        transform: `translate(${x}px, ${y}px)`,
        width: w,
        height: h,
        viewTransitionName: vtName,
      };

  return (
    <>
      <div
        className={squareClass}
        style={squareStyle}
        onMouseDown={(e) => {
          e.stopPropagation();
          onSelect(tile.id);
        }}
        {...{ [SQUARE_ID_ATTR]: tile.id }}
      >
        {/* Pill menu — centered on the top edge, half above the window */}
        {!fullscreen && (
          <div
            className="tile-pill"
            onMouseDown={(e) => e.stopPropagation()}
            onPointerDown={(e) => e.stopPropagation()}
          >
            <button className="tile-pill__btn" onClick={handleCopyLink} title="Copy link">
              <IconLink />
            </button>
            <button className="tile-pill__btn" onClick={handleRefresh} title="Refresh">
              <IconRefresh />
            </button>
            <button className="tile-pill__btn" onClick={handleViewSource} title="View source">
              <IconCode />
            </button>
            <button
              className={`tile-pill__btn${locked ? " tile-pill__btn--active" : ""}`}
              onClick={handleLock}
              title={locked ? "Unlock" : "Lock"}
            >
              <IconLock locked={locked} />
            </button>
            {isDoc && (
              <button
                className="tile-pill__btn"
                onClick={handleToggleWidth}
                title={docWidth === "full" ? "Make half-width" : "Make full-width"}
              >
                <IconWidth half={docWidth === "full"} />
              </button>
            )}
          </div>
        )}

        {/* Drag dots — outside the square to the left, floating */}
        {!fullscreen && (
          <div
            className="square__drag-dots"
            onPointerDown={
              canDrag ? beginDrag : canReorder ? beginReorder : undefined
            }
          >
            {Array.from({ length: 6 }, (_, i) => (
              <span key={i} className="square__drag-dot" />
            ))}
          </div>
        )}

        {/* Title bar */}
        <div
          className="square__bar"
          onPointerDown={
            canDrag ? beginDrag : canReorder ? beginReorder : undefined
          }
          style={color ? { background: color.soft } : undefined}
        >
          {titleLabel && (
            <div className="square__name">
              <span>{titleLabel}</span>
            </div>
          )}
        </div>

        {/* Window controls — right side */}
        <button
          className="square__minimize"
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => { e.stopPropagation(); onMinimize(tile.id); }}
          aria-label="minimize"
        />
        <button
          className={`square__fullscreen${fullscreen ? " square__fullscreen--on" : ""}`}
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => { e.stopPropagation(); onFullscreen(tile.id); }}
          aria-label={fullscreen ? "exit fullscreen" : "fullscreen"}
        />
        <button
          className="square__close"
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => { e.stopPropagation(); onClose(tile.id); }}
          aria-label="close"
        />

        {/* Content */}
        <div className="square__body">
          {canRender && (
            <iframe
              ref={iframeRef}
              className="square__iframe"
              sandbox="allow-scripts allow-same-origin"
              src={src!}
              title={titleLabel ?? tile.entity}
            />
          )}
        </div>

        {!canRender && (
          <button
            className="square__pick"
            onClick={(e) => { e.stopPropagation(); onOpenPicker(tile.id); }}
          >
            <span className="square__pick-plus">+</span>
            <span className="square__pick-label">Choose artifact</span>
          </button>
        )}

        {pickerOpen && (
          <div className="picker-anchor">
            <EntityPicker
              initialEntity={tile.entity}
              onPick={onPick}
              onClose={onClosePicker}
            />
          </div>
        )}

        {/* Resize handles */}
        {canResize && EDGES.map((edge) => (
          <div
            key={edge}
            className={`square__edge square__edge--${edge}`}
            onPointerDown={beginResize(edge)}
            aria-label={`resize ${edge}`}
          />
        ))}
        {canResize && CORNERS.map((corner) => (
          <div
            key={corner}
            className={`square__handle square__handle--${corner}`}
            onPointerDown={beginResize(corner)}
            aria-label={`resize ${corner}`}
          />
        ))}
      </div>

      {/* Drag/resize ghost cells (canvas mode only) */}
      {!fullscreen && !isDoc && ghost &&
        Array.from({ length: ghost.w * ghost.h }, (_, i) => {
          const c = i % ghost.w;
          const r = Math.floor(i / ghost.w);
          return (
            <div
              key={i}
              className="selection-cell"
              style={{
                transform: `translate(${ghost.col * cellSize + c * widgetSize}px, ${ghost.row * cellSize + r * widgetSize}px)`,
                width: widgetSize,
                height: widgetSize,
              }}
            />
          );
        })}
    </>
  );
}
