import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { resolveName } from "../lib/resolveName";
import { isTileMessage, SQUARE_ID_ATTR, type TileMessage } from "../lib/tile-messages";
import {
  insert,
  insetRect,
  layout,
  makeLeaf,
  removeLeaf,
  updateLeaf,
  type Leaf,
  type Node,
  type Rect,
} from "../lib/layout";
import { computeZones, RAIL_PX, type Zone } from "../lib/zones";
import { RepoContext } from "../context";
import type { PickPayload } from "./EntityPicker";
import { Square } from "./Square";
import { EdgeRails } from "./EdgeRails";
import { Rail } from "./Rail";

const TILE_INSET = 6;
const PREVIEW_LEAF_ID = "__preview__";
// How far existing tiles drift toward their post-insert position
// when a rail zone is hovered. The user spec is "slightly" — full
// preview makes the layout feel committed before the click; this
// hint plus the blue ghost is enough to communicate the outcome.
// Kept small (1/10) so the gap that opens between the rail and the
// drifted tile stays narrow — the gap is also covered by an
// expanded hit rect on the hovered zone (see expandedZones below)
// so a cursor that strays into it doesn't lose the preview.
const PREVIEW_HINT = 0.1;

function blendRect(a: Rect, b: Rect, t: number): Rect {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
    w: a.w + (b.w - a.w) * t,
    h: a.h + (b.h - a.h) * t,
  };
}

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
  const wrap = useElementSize(wrapperRef);
  const repo = useContext(RepoContext);

  const [tree, setTree] = useState<Node | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pickerForId, setPickerForId] = useState<string | null>(null);
  const [hoveredZone, setHoveredZone] = useState<Zone | null>(null);
  // Minimized leaves live outside the tree as a flat list. The
  // tree only knows about visible tiles; restoring re-inserts at
  // the right edge of root (same shape as a right-rail click,
  // minus the picker step since the leaf already has an entity).
  const [minimized, setMinimized] = useState<Leaf[]>([]);
  // Fullscreen is a transient view-state, not a tree mutation —
  // when set, we render only that leaf and skip the rails. Toggle
  // off restores the regular tree layout.
  const [fullscreenId, setFullscreenId] = useState<string | null>(null);

  // Stage covers the whole wrapper; the inner grid box sits with
  // RAIL_PX padding on all four sides so rails fit at the edges.
  // The host page parks small corner affordances (the drawer
  // chevron at top-left, the share button at top-right) inside
  // the same strip — both are sized to fit within RAIL_PX × RAIL_PX
  // so they don't extend into tile territory. Both dims have a
  // hard floor so layout math doesn't go negative on first paint.
  const stageW = Math.max(0, wrap.w);
  const stageH = Math.max(0, wrap.h);
  const gridW = Math.max(0, stageW - 2 * RAIL_PX);
  const gridH = Math.max(0, stageH - 2 * RAIL_PX);
  const gridOriginX = RAIL_PX;
  const gridOriginY = RAIL_PX;
  const gridRect: Rect = useMemo(
    () => ({ x: 0, y: 0, w: gridW, h: gridH }),
    [gridW, gridH],
  );

  // Preview pipeline: when a rail zone is hovered, synthesise an
  // insert into the tree with a sentinel leaf and lay that out.
  // The resulting rects are what we actually render — so the
  // existing tiles glide to their post-insert positions via CSS
  // transitions, and the sentinel rect drives the blue ghost.
  const currentRects = useMemo(() => (tree ? layout(tree, gridRect) : []), [tree, gridRect]);

  const previewTree = useMemo<Node | null>(() => {
    if (!tree || !hoveredZone) return null;
    const sentinel: Leaf = { kind: "leaf", id: PREVIEW_LEAF_ID };
    return insert(tree, hoveredZone.edge, hoveredZone.targetNodeId, sentinel);
  }, [tree, hoveredZone]);

  const previewRects = useMemo(
    () => (previewTree ? layout(previewTree, gridRect) : []),
    [previewTree, gridRect],
  );

  // Per-leaf displayed rect: blend toward the post-insert layout
  // by PREVIEW_HINT while a zone is hovered. Without a preview,
  // each leaf sits at its current rect.
  const displayedLeaves = useMemo(() => {
    if (!tree) return [];
    if (!previewTree) return currentRects;
    const cur = new Map(currentRects.map((lr) => [lr.id, lr.rect]));
    return previewRects.map((lr) => {
      if (lr.id === PREVIEW_LEAF_ID) return lr;
      const c = cur.get(lr.id);
      if (!c) return lr;
      return { ...lr, rect: blendRect(c, lr.rect, PREVIEW_HINT) };
    });
  }, [tree, previewTree, currentRects, previewRects]);

  // Ghost shows the new tile at its full final position so the
  // outcome is unambiguous, even though the existing tiles have
  // only drifted partway. We extend the ghost on the side that
  // matches the hovered rail so the dashed outline reaches the
  // stage edge — without this it stops `gridOrigin + TILE_INSET`
  // (=34px) in from the edge and reads as a visible gap between
  // the rail the user is hovering and the preview.
  const ghostRect = useMemo<Rect | null>(() => {
    if (!previewTree) return null;
    const found = previewRects.find((lr) => lr.id === PREVIEW_LEAF_ID);
    if (!found) return null;
    let r = insetRect(found.rect, TILE_INSET);
    const dx = gridOriginX + TILE_INSET;
    const dy = gridOriginY + TILE_INSET;
    switch (hoveredZone?.edge) {
      case "top":
        r = { ...r, y: r.y - dy, h: r.h + dy };
        break;
      case "bottom":
        r = { ...r, h: r.h + dy };
        break;
      case "left":
        r = { ...r, x: r.x - dx, w: r.w + dx };
        break;
      case "right":
        r = { ...r, w: r.w + dx };
        break;
    }
    return r;
  }, [previewTree, previewRects, hoveredZone, gridOriginX, gridOriginY]);

  const zones = useMemo(() => {
    if (!tree) return [];
    return computeZones(tree, {
      gridX: gridOriginX,
      gridY: gridOriginY,
      gridW,
      gridH,
    });
  }, [tree, gridOriginX, gridOriginY, gridW, gridH]);

  // While a rail zone is hovered, grow that zone's hit rect to
  // swallow the gap that opens between the rail and the drifted
  // tile. Without this, pulling the cursor back even slightly
  // (e.g. the user's natural micro-correction after the preview
  // appears) exits the rail and snaps the layout shut, then a
  // small forward motion re-enters it — that's the "flicker" the
  // user reported. The visible rail still uses its original size
  // (CSS controls the highlight); the hit rect is what changes.
  const expandedZones = useMemo<Zone[]>(() => {
    if (!hoveredZone) return zones;
    const others = displayedLeaves.filter((lr) => lr.id !== PREVIEW_LEAF_ID);
    if (others.length === 0) return zones;

    const edge = hoveredZone.edge;
    let coverTo: number;
    if (edge === "top") {
      let minY = Infinity;
      for (const lr of others) {
        const stageY = lr.rect.y + gridOriginY + TILE_INSET;
        if (stageY < minY) minY = stageY;
      }
      coverTo = minY;
    } else if (edge === "bottom") {
      let maxY = -Infinity;
      for (const lr of others) {
        const stageB = lr.rect.y + lr.rect.h + gridOriginY - TILE_INSET;
        if (stageB > maxY) maxY = stageB;
      }
      coverTo = maxY;
    } else if (edge === "left") {
      let minX = Infinity;
      for (const lr of others) {
        const stageX = lr.rect.x + gridOriginX + TILE_INSET;
        if (stageX < minX) minX = stageX;
      }
      coverTo = minX;
    } else {
      let maxX = -Infinity;
      for (const lr of others) {
        const stageR = lr.rect.x + lr.rect.w + gridOriginX - TILE_INSET;
        if (stageR > maxX) maxX = stageR;
      }
      coverTo = maxX;
    }

    return zones.map((z) => {
      if (z.id !== hoveredZone.id) return z;
      const r = { ...z.rect };
      if (edge === "top") {
        r.h = Math.max(r.h, coverTo - r.y);
      } else if (edge === "bottom") {
        const oldBottom = r.y + r.h;
        r.y = Math.min(r.y, coverTo);
        r.h = oldBottom - r.y;
      } else if (edge === "left") {
        r.w = Math.max(r.w, coverTo - r.x);
      } else {
        const oldRight = r.x + r.w;
        r.x = Math.min(r.x, coverTo);
        r.w = oldRight - r.x;
      }
      return { ...z, rect: r };
    });
  }, [zones, hoveredZone, displayedLeaves, gridOriginX, gridOriginY]);

  const handleEmptyClick = useCallback(() => {
    const leaf = makeLeaf();
    setTree(leaf);
    setSelectedId(leaf.id);
    setPickerForId(leaf.id);
  }, []);

  const handleInsert = useCallback((zone: Zone) => {
    setTree((cur) => {
      if (!cur) return cur;
      const leaf = makeLeaf();
      // Use the same sentinel-replacement semantics as preview to
      // keep insert paths identical: the new leaf takes the slot
      // the preview leaf would have taken.
      setSelectedId(leaf.id);
      setPickerForId(leaf.id);
      return insert(cur, zone.edge, zone.targetNodeId, leaf);
    });
    setHoveredZone(null);
  }, []);

  const handleSelect = useCallback((id: string) => {
    setSelectedId(id);
  }, []);

  const handleClose = useCallback((id: string) => {
    setTree((cur) => (cur ? removeLeaf(cur, id) : cur));
    setMinimized((cur) => cur.filter((l) => l.id !== id));
    setSelectedId((cur) => (cur === id ? null : cur));
    setPickerForId((cur) => (cur === id ? null : cur));
    setFullscreenId((cur) => (cur === id ? null : cur));
  }, []);

  const handleMinimize = useCallback((id: string) => {
    setTree((cur) => {
      if (!cur) return cur;
      // Snapshot the leaf before we collapse it out of the tree.
      // The leaf gets pushed into `minimized` from this same setter
      // so the read and the remove stay consistent across renders.
      let captured: Leaf | null = null;
      const stripped = (function strip(node: Node): Node | null {
        if (node.kind === "leaf") {
          if (node.id === id) {
            captured = node;
            return null;
          }
          return node;
        }
        const next: Node[] = [];
        for (const c of node.children) {
          const r = strip(c);
          if (r) next.push(r);
        }
        if (next.length === 0) return null;
        if (next.length === 1) return next[0]!;
        return { ...node, children: next };
      })(cur);
      if (captured) {
        setMinimized((m) => [...m, captured!]);
      }
      return stripped;
    });
    setSelectedId((cur) => (cur === id ? null : cur));
    setPickerForId((cur) => (cur === id ? null : cur));
    setFullscreenId((cur) => (cur === id ? null : cur));
  }, []);

  const handleRestore = useCallback((id: string) => {
    setMinimized((cur) => {
      const leaf = cur.find((l) => l.id === id);
      if (!leaf) return cur;
      setTree((t) => {
        if (!t) return leaf;
        // Right-edge append: same shape as a right-rail click.
        return insert(t, "right", t.id, leaf);
      });
      setSelectedId(leaf.id);
      return cur.filter((l) => l.id !== id);
    });
  }, []);

  const handleFullscreen = useCallback((id: string) => {
    setFullscreenId((cur) => (cur === id ? null : id));
    setSelectedId(id);
    setHoveredZone(null);
  }, []);

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
      setTree((cur) =>
        cur
          ? updateLeaf(cur, id, {
              entity: payload.entity,
              name: payload.name,
              branch: payload.branch,
            })
          : cur,
      );
      setPickerForId(null);
    },
    [pickerForId],
  );

  // Listen for postMessages from artifact iframes. Same wire
  // contract as before (`lib/tile-messages.ts`); the only change is
  // we mutate the tree's leaf instead of an entry in a flat list.
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

      setTree((cur) =>
        cur
          ? updateLeaf(cur, squareId, {
              entity,
              name,
              branch: targetBranch,
            })
          : cur,
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

  // Sentinel filtered out so we don't try to render the preview
  // leaf as a real tile (it has no entity and the picker would
  // mis-target it).
  const renderableLeaves = displayedLeaves.filter((lr) => lr.id !== PREVIEW_LEAF_ID);

  // Fullscreen overrides the normal layout: one leaf takes the
  // whole grid rect, others are hidden, rails and ghost suppressed.
  // If the fullscreened leaf has been removed from the tree (close
  // or minimize), we fall back to the normal flow — handlers above
  // already clear `fullscreenId` in those paths.
  const fullscreenLeaf =
    fullscreenId != null
      ? renderableLeaves.find((lr) => lr.id === fullscreenId)?.leaf ?? null
      : null;
  const inFullscreen = fullscreenLeaf != null;

  return (
    <div ref={wrapperRef} className="grid-wrapper">
      <div
        className="grid-stage"
        style={{ width: stageW, height: stageH }}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) setSelectedId(null);
        }}
      >
        <div
          className={`grid${tree ? "" : " grid--empty"}`}
          style={{
            transform: `translate(${gridOriginX}px, ${gridOriginY}px)`,
            width: gridW,
            height: gridH,
          }}
          onClick={(e) => {
            if (!tree && e.target === e.currentTarget) handleEmptyClick();
          }}
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setSelectedId(null);
          }}
        >
          {!tree && <div className="grid__empty">Click anywhere to create the first tile</div>}
        </div>
        {inFullscreen ? (
          <Square
            key={fullscreenLeaf!.id}
            leaf={fullscreenLeaf!}
            rect={insetRect(
              { x: gridOriginX, y: gridOriginY, w: gridW, h: gridH },
              TILE_INSET,
            )}
            selected={fullscreenLeaf!.id === selectedId}
            pickerOpen={fullscreenLeaf!.id === pickerForId}
            fullscreen={true}
            onSelect={handleSelect}
            onClose={handleClose}
            onMinimize={handleMinimize}
            onFullscreen={handleFullscreen}
            onOpenPicker={handleOpenPicker}
            onPick={handlePick}
            onClosePicker={handleClosePicker}
          />
        ) : (
          renderableLeaves.map((lr) => {
            const positioned = insetRect(
              {
                x: lr.rect.x + gridOriginX,
                y: lr.rect.y + gridOriginY,
                w: lr.rect.w,
                h: lr.rect.h,
              },
              TILE_INSET,
            );
            return (
              <Square
                key={lr.leaf.id}
                leaf={lr.leaf}
                rect={positioned}
                selected={lr.leaf.id === selectedId}
                pickerOpen={lr.leaf.id === pickerForId}
                fullscreen={false}
                onSelect={handleSelect}
                onClose={handleClose}
                onMinimize={handleMinimize}
                onFullscreen={handleFullscreen}
                onOpenPicker={handleOpenPicker}
                onPick={handlePick}
                onClosePicker={handleClosePicker}
              />
            );
          })
        )}
        {!inFullscreen && ghostRect && (
          <div
            className="insert-ghost"
            style={{
              transform: `translate(${ghostRect.x + gridOriginX}px, ${ghostRect.y + gridOriginY}px)`,
              width: ghostRect.w,
              height: ghostRect.h,
            }}
          />
        )}
        {!inFullscreen && tree && (
          <EdgeRails
            zones={expandedZones}
            hoveredId={hoveredZone?.id ?? null}
            onEnter={setHoveredZone}
            onLeave={() => setHoveredZone(null)}
            onCommit={handleInsert}
          />
        )}
        <Rail leaves={minimized} onRestore={handleRestore} />
      </div>
    </div>
  );
}
