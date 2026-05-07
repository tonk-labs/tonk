// Compute the hit-zones for the four edge rails outside the grid.
//
// Rule: a rail's segmentation reflects the *direct children* of the
// root, only when the rail's axis matches the root's split axis:
//
//   - top/bottom rails are horizontal → segmented by columns (N+1
//     children of a col-split root spawn 2N-1 zones along the rail
//     that alternate child / between-children-gap).
//   - left/right rails are vertical → segmented by rows of a row-
//     split root, same way.
//   - Any other case (root is a leaf, or rail axis is parallel to
//     root's split axis) collapses to a single rail zone targeting
//     the root.
//
// Between-children zones target the parent (root) so a click in
// the gap wraps the whole root in a perpendicular split. Child
// zones target the corresponding root child so a click wraps just
// that branch.

import type { Edge, Node, Rect } from "./layout";

export type Zone = {
  id: string;
  edge: Edge;
  // Node the insert will operate against. Either the root or a
  // direct child of the root (we never recurse deeper — the
  // "outermost" rule).
  targetNodeId: string;
  // Hit area in grid-wrapper-local coords (relative to the wrapper
  // div, *not* the inner grid box). Rails sit outside the grid box.
  rect: Rect;
};

// Width of the rail hit zone (and the gap between the inner grid
// box and the stage edge). 40px is at the low end of practical
// mouse-target guidelines (Apple HIG suggests 44, Material 48);
// we get away with it because the hovered rail expands toward
// the drifted tile (see expandedZones in Grid.tsx) so the
// effective hit area is much larger after the first pixel of
// hover. The same strip houses the corner affordances on the
// host page (drawer chevron at top-left, share at top-right),
// both sized to fit inside RAIL_PX × RAIL_PX.
export const RAIL_PX = 40;
// Width of the between-children zone along the rail. Pulled from
// the adjacent child zones, not added on top, so the rail length
// stays equal to the grid side length.
const BETWEEN_PX = 40;

export type RailGeometry = {
  // Origin of the inner grid box within the wrapper. Rails are
  // positioned outside this box.
  gridX: number;
  gridY: number;
  gridW: number;
  gridH: number;
};

export function computeZones(tree: Node, geom: RailGeometry): Zone[] {
  const zones: Zone[] = [];
  for (const edge of ["top", "right", "bottom", "left"] as const) {
    zones.push(...zonesForEdge(tree, edge, geom));
  }
  return zones;
}

function zonesForEdge(tree: Node, edge: Edge, geom: RailGeometry): Zone[] {
  const matchingAxis = edge === "top" || edge === "bottom" ? "col" : "row";

  // Top/bottom rails own the four corners so that *every* pixel
  // along the stage edge fires some zone — moving the cursor to
  // the very corner of the window should activate, not land in a
  // dead corner gap. Left/right rails stay flush with the inner
  // grid in the perpendicular axis (their corners are already
  // covered by top/bottom). Extension length = the perpendicular
  // rail's thickness (= the X-inset, since left/right insets are
  // symmetric); the top inset can differ (asymmetric layout) and
  // doesn't affect this number.
  const cornerExt = edge === "top" || edge === "bottom" ? geom.gridX : 0;

  // Single zone covering the whole rail when root isn't split
  // along the perpendicular axis.
  if (tree.kind !== "split" || tree.axis !== matchingAxis) {
    return [
      {
        id: `${edge}-root`,
        edge,
        targetNodeId: tree.id,
        rect: railRect(edge, -cornerExt, sideLength(edge, geom) + 2 * cornerExt, geom),
      },
    ];
  }

  // Tree is split along the matching axis: alternate child / gap
  // zones along the rail. For N children we emit 2N-1 zones.
  const N = tree.children.length;
  const total = sideLength(edge, geom);
  const seg = total / N;
  const half = BETWEEN_PX / 2;

  const out: Zone[] = [];
  for (let i = 0; i < N; i++) {
    const cStart = i * seg;
    const cEnd = (i + 1) * seg;
    // Pull `half` off each side that has a between-zone neighbour
    // (the outer edges keep the full extent so corner clicks still
    // target the outermost child). The first/last zones also
    // extend by `cornerExt` so they cover the actual corner pixels.
    const zStart = (i === 0 ? cStart - cornerExt : cStart + half);
    const zEnd = (i === N - 1 ? cEnd + cornerExt : cEnd - half);
    out.push({
      id: `${edge}-c${i}`,
      edge,
      targetNodeId: tree.children[i]!.id,
      rect: railRect(edge, zStart, zEnd - zStart, geom),
    });
    if (i < N - 1) {
      out.push({
        id: `${edge}-gap${i}`,
        edge,
        targetNodeId: tree.id,
        rect: railRect(edge, cEnd - half, BETWEEN_PX, geom),
      });
    }
  }
  return out;
}

function sideLength(edge: Edge, geom: RailGeometry): number {
  return edge === "top" || edge === "bottom" ? geom.gridW : geom.gridH;
}

function railRect(edge: Edge, start: number, length: number, geom: RailGeometry): Rect {
  // Rail thickness derives from the actual grid inset on each side
  // rather than the RAIL_PX constant, so the layout can use an
  // asymmetric top inset (smaller, to sit flush with the host
  // page's notches) without producing a negative-y top rail.
  switch (edge) {
    case "top":
      return { x: geom.gridX + start, y: 0, w: length, h: geom.gridY };
    case "bottom":
      return { x: geom.gridX + start, y: geom.gridY + geom.gridH, w: length, h: RAIL_PX };
    case "left":
      return { x: 0, y: geom.gridY + start, w: geom.gridX, h: length };
    case "right":
      return { x: geom.gridX + geom.gridW, y: geom.gridY + start, w: RAIL_PX, h: length };
  }
}
