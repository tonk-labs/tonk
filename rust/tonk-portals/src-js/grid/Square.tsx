import { useContext } from "react";
import { EntityPicker, type PickPayload } from "./EntityPicker";
import { ArtifactMenu } from "./ArtifactMenu";
import { colorForArtifact } from "../lib/color";
import { SQUARE_ID_ATTR } from "../lib/tile-messages";
import type { Leaf, Rect } from "../lib/layout";
import { HostContext, RepoContext } from "../context";

type Props = {
  leaf: Leaf;
  rect: Rect;
  selected: boolean;
  pickerOpen: boolean;
  fullscreen: boolean;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onMinimize: (id: string) => void;
  onFullscreen: (id: string) => void;
  onOpenPicker: (id: string) => void;
  onPick: (payload: PickPayload) => void;
  onClosePicker: () => void;
};

function tileSrc(repo: string, host: string, entity: string, branch: string): string {
  return `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(branch)}/host/${encodeURIComponent(host)}/${entity}`;
}

export function Square({
  leaf,
  rect,
  selected,
  pickerOpen,
  fullscreen,
  onSelect,
  onClose,
  onMinimize,
  onFullscreen,
  onOpenPicker,
  onPick,
  onClosePicker,
}: Props) {
  const repo = useContext(RepoContext);
  const host = useContext(HostContext);

  const color = colorForArtifact(leaf);
  const branch = leaf.branch ?? "main";
  const titleLabel = leaf.name ?? leaf.entity;
  const canRender = !!repo && !!host && !!leaf.entity;

  return (
    <div
      className={`square${selected ? " square--selected" : ""}`}
      style={{
        transform: `translate(${rect.x}px, ${rect.y}px)`,
        width: rect.w,
        height: rect.h,
      }}
      onMouseDown={(e) => {
        e.stopPropagation();
        onSelect(leaf.id);
      }}
      {...{ [SQUARE_ID_ATTR]: leaf.id }}
    >
      <div
        className="square__bar"
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
          onClose(leaf.id);
        }}
        aria-label="close"
      />
      <button
        className="square__minimize"
        onPointerDown={(e) => e.stopPropagation()}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onMinimize(leaf.id);
        }}
        aria-label="minimize"
      />
      <button
        className={`square__fullscreen${fullscreen ? " square__fullscreen--on" : ""}`}
        onPointerDown={(e) => e.stopPropagation()}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onFullscreen(leaf.id);
        }}
        aria-label={fullscreen ? "exit fullscreen" : "fullscreen"}
      />
      <ArtifactMenu />
      <div className="square__body">
        {canRender && (
          <iframe
            className="square__iframe"
            sandbox="allow-scripts allow-same-origin"
            src={tileSrc(repo, host, leaf.entity!, branch)}
            title={titleLabel ?? leaf.entity}
          />
        )}
      </div>
      {!canRender && (
        // Pick button sits *outside* the body so it covers the
        // whole tile (including the 28px chrome bar). That way the
        // "+" / "Choose artifact" centers on the tile's geometric
        // center, and a click anywhere on the empty tile opens the
        // picker. Chrome buttons (close/min/fullscreen/menu) keep
        // their z-index: 5 so they still take precedence over the
        // pick's flat z-index when the cursor is on top of them.
        <button
          className="square__pick"
          onClick={(e) => {
            e.stopPropagation();
            onOpenPicker(leaf.id);
          }}
        >
          <span className="square__pick-plus">+</span>
          <span className="square__pick-label">Choose artifact</span>
        </button>
      )}
      {pickerOpen && (
        <div className="picker-anchor">
          <EntityPicker
            initialEntity={leaf.entity}
            onPick={onPick}
            onClose={onClosePicker}
          />
        </div>
      )}
    </div>
  );
}
