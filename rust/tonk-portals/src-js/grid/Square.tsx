import { useContext } from "react";
import type { Square as SquareT } from "../types";
import { WIDGET_SPAN, labelFor, spanForWidgets } from "../lib/presets";
import { useDrag } from "../lib/useDrag";
import { type Corner, type Edge, useResize } from "../lib/useResize";
import { EntityPicker, type PickPayload } from "./EntityPicker";
import { ArtifactMenu } from "./ArtifactMenu";
import { colorForArtifact } from "../lib/color";
import { HostContext, RepoContext } from "../context";

type Props = {
  square: SquareT;
  cellSize: number;
  maxW: number;
  maxH: number;
  selected: boolean;
  pickerOpen: boolean;
  onSelect: (id: string) => void;
  onResize: (id: string, next: { w: number; h: number; col: number; row: number }) => void;
  onMove: (id: string, next: { col: number; row: number }) => void;
  onMinimize: (id: string) => void;
  onFullscreen: (id: string) => void;
  onClose: (id: string) => void;
  onOpenPicker: (id: string) => void;
  onPick: (payload: PickPayload) => void;
  onClosePicker: () => void;
};

const CORNERS: Corner[] = ["tl", "tr", "bl", "br"];
const EDGES: Edge[] = ["t", "r", "b", "l"];

function tileSrc(repo: string, host: string, entity: string, branch: string): string {
  return `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(branch)}/host/${encodeURIComponent(host)}/${entity}`;
}

export function Square({
  square,
  cellSize,
  maxW,
  maxH,
  selected,
  pickerOpen,
  onSelect,
  onResize,
  onMove,
  onMinimize,
  onFullscreen,
  onClose,
  onOpenPicker,
  onPick,
  onClosePicker,
}: Props) {
  const repo = useContext(RepoContext);
  const host = useContext(HostContext);

  const widthPx = spanForWidgets(square.w) * cellSize;
  const heightPx = spanForWidgets(square.h) * cellSize;
  const x = square.col * cellSize;
  const y = square.row * cellSize;

  const { ghost: resizeGhost, beginResize } = useResize({
    cellSize,
    startW: square.w,
    startH: square.h,
    startCol: square.col,
    startRow: square.row,
    maxW,
    maxH,
    onCommit: (next) => onResize(square.id, next),
  });

  const { ghost: dragGhost, beginDrag } = useDrag({
    cellSize,
    startCol: square.col,
    startRow: square.row,
    w: square.w,
    h: square.h,
    maxW,
    maxH,
    onCommit: (next) => onMove(square.id, next),
  });

  const widgetSize = WIDGET_SPAN * cellSize;
  const ghost = resizeGhost
    ? { ...resizeGhost }
    : dragGhost
      ? { col: dragGhost.col, row: dragGhost.row, w: square.w, h: square.h }
      : null;
  const isActive = !!ghost;
  const color = colorForArtifact(square);
  const branch = square.branch ?? "main";
  const titleLabel = square.name ?? square.entity;
  const canRender = !!repo && !!host && !!square.entity;

  return (
    <>
      <div
        className={`square${selected ? " square--selected" : ""}${isActive ? " square--active" : ""}`}
        style={{
          transform: `translate(${x}px, ${y}px)`,
          width: widthPx,
          height: heightPx,
        }}
        onMouseDown={(e) => {
          e.stopPropagation();
          onSelect(square.id);
        }}
      >
        <div
          className="square__bar"
          onPointerDown={beginDrag}
          style={color ? { background: color.soft } : undefined}
        >
          {titleLabel && (
            <div className="square__name">
              <span>{titleLabel}</span>
            </div>
          )}
        </div>
        <button
          className="square__close"
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onClose(square.id);
          }}
          aria-label="close"
        />
        <button
          className="square__fullscreen"
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onFullscreen(square.id);
          }}
          aria-label="fullscreen"
        />
        <button
          className="square__minimize"
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onMinimize(square.id);
          }}
          aria-label="minimize"
        />
        <ArtifactMenu />
        <div className="square__body">
          {canRender ? (
            <iframe
              className="square__iframe"
              sandbox="allow-scripts allow-same-origin"
              src={tileSrc(repo, host, square.entity!, branch)}
              title={titleLabel ?? square.entity}
            />
          ) : (
            <button
              className="square__pick"
              onClick={(e) => {
                e.stopPropagation();
                onOpenPicker(square.id);
              }}
            >
              <span className="square__pick-plus">+</span>
              <span className="square__pick-label">Choose artifact</span>
              <span className="square__pick-size">{labelFor(square.w, square.h)}</span>
            </button>
          )}
        </div>
        {pickerOpen && (
          <div className="picker-anchor">
            <EntityPicker
              initialEntity={square.entity}
              initialBranch={square.branch}
              onPick={onPick}
              onClose={onClosePicker}
            />
          </div>
        )}
        {EDGES.map((edge) => (
          <div
            key={edge}
            className={`square__edge square__edge--${edge}`}
            onPointerDown={beginResize(edge)}
            aria-label={`resize ${edge}`}
          />
        ))}
        {CORNERS.map((corner) => (
          <div
            key={corner}
            className={`square__handle square__handle--${corner}`}
            onPointerDown={beginResize(corner)}
            aria-label={`resize ${corner}`}
          />
        ))}
      </div>
      {ghost &&
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
