// Tree-based tile layout for the portals grid.
//
// A layout is a tree of `Split` nodes (axis = "col" | "row", with N
// children) and `Leaf` nodes (one per tile). Leaves carry the
// artifact reference; splits divide their box equally among their
// children. Insertion always operates on a known target node and
// either grows a sibling list (when target's parent already splits
// along the requested axis) or wraps the target in a new split.
//
// The shell never targets nodes deeper than root.children — that's
// the "outermost" rule the user agreed to: a top-left rail click
// affects the left column container of the root, even if that
// column happens to itself be split into rows. Going deeper would
// make the rail behaviour unpredictable.

export type Edge = "top" | "right" | "bottom" | "left";
export type Axis = "col" | "row";

export type Leaf = {
  kind: "leaf";
  id: string;
  entity?: string;
  branch?: string;
  name?: string;
};

export type Split = {
  kind: "split";
  id: string;
  axis: Axis;
  children: Node[];
};

export type Node = Leaf | Split;

export type Rect = { x: number; y: number; w: number; h: number };

export type LeafRect = { id: string; leaf: Leaf; rect: Rect };

let counter = 1;
const nextId = (prefix: string) => `${prefix}-${counter++}`;

export function makeLeaf(init?: Partial<Omit<Leaf, "kind" | "id">>): Leaf {
  return {
    kind: "leaf",
    id: nextId("lf"),
    ...init,
  };
}

export function axisOf(edge: Edge): Axis {
  return edge === "top" || edge === "bottom" ? "row" : "col";
}

export function isPrepend(edge: Edge): boolean {
  return edge === "top" || edge === "left";
}

// ---------- queries ----------

export function findNode(tree: Node, id: string): Node | null {
  if (tree.id === id) return tree;
  if (tree.kind === "leaf") return null;
  for (const child of tree.children) {
    const hit = findNode(child, id);
    if (hit) return hit;
  }
  return null;
}

export function findParent(tree: Node, childId: string): Split | null {
  if (tree.kind === "leaf") return null;
  for (const child of tree.children) {
    if (child.id === childId) return tree;
    const deeper = findParent(child, childId);
    if (deeper) return deeper;
  }
  return null;
}

export function allLeaves(tree: Node): Leaf[] {
  if (tree.kind === "leaf") return [tree];
  return tree.children.flatMap(allLeaves);
}

// ---------- mutations (pure, return new tree) ----------

function withChildren(split: Split, children: Node[]): Split {
  return { ...split, children };
}

function replaceNode(tree: Node, id: string, replacement: Node): Node {
  if (tree.id === id) return replacement;
  if (tree.kind === "leaf") return tree;
  return withChildren(
    tree,
    tree.children.map((c) => replaceNode(c, id, replacement)),
  );
}

function makeSplit(axis: Axis, children: Node[]): Split {
  return { kind: "split", id: nextId("sp"), axis, children };
}

// Core operation: place `newLeaf` adjacent to `targetId` along `edge`.
//
// If the target's parent already splits along the matching axis,
// just insert the leaf as a sibling in the right slot (this is the
// "2-up → 3-up" path). Otherwise, wrap the target in a new split
// of that axis, with the new leaf on the prepend/append side.
//
// Special case: if `targetId === tree.id` AND the root itself is a
// split with the matching axis, we still extend the root's child
// list rather than wrapping (otherwise a right-rail click on a
// 2-column root would create a 2-column root containing a
// 2-column root, which isn't what the user expects).
export function insert(tree: Node, edge: Edge, targetId: string, newLeaf: Leaf): Node {
  const ax = axisOf(edge);
  const prepend = isPrepend(edge);

  const target = findNode(tree, targetId);
  if (!target) return tree;

  // Root-level append/prepend: target is the root, root is a split
  // along the matching axis → just add to root.children.
  if (target.id === tree.id && target.kind === "split" && target.axis === ax) {
    const next = prepend ? [newLeaf, ...target.children] : [...target.children, newLeaf];
    return withChildren(target, next);
  }

  // Target is itself a split along the matching axis (rare in v1
  // since we only ever target root or root.children, but handled
  // for completeness): extend its children.
  if (target.kind === "split" && target.axis === ax) {
    const extended = withChildren(
      target,
      prepend ? [newLeaf, ...target.children] : [...target.children, newLeaf],
    );
    return replaceNode(tree, target.id, extended);
  }

  // Otherwise wrap the target in a new split of the requested axis.
  const wrap = makeSplit(ax, prepend ? [newLeaf, target] : [target, newLeaf]);
  if (target.id === tree.id) return wrap;
  return replaceNode(tree, target.id, wrap);
}

// Remove a leaf. If its parent ends up with one child, collapse the
// parent to that child (recursively up). If the only leaf is
// removed, return null — the caller chooses what to do (we render
// the empty grid).
export function removeLeaf(tree: Node, leafId: string): Node | null {
  if (tree.kind === "leaf") {
    return tree.id === leafId ? null : tree;
  }
  const filtered: Node[] = [];
  for (const child of tree.children) {
    const next = removeLeaf(child, leafId);
    if (next) filtered.push(next);
  }
  if (filtered.length === 0) return null;
  if (filtered.length === 1) return filtered[0]!; // collapse single-child split
  return withChildren(tree, filtered);
}

export function updateLeaf(
  tree: Node,
  leafId: string,
  patch: Partial<Omit<Leaf, "kind" | "id">>,
): Node {
  if (tree.kind === "leaf") {
    if (tree.id !== leafId) return tree;
    return { ...tree, ...patch };
  }
  return withChildren(
    tree,
    tree.children.map((c) => updateLeaf(c, leafId, patch)),
  );
}

// ---------- layout ----------

// Recursively assign pixel rects. Splits divide their rect equally
// among children; gaps are *not* applied here — the grid renderer
// adds a small inset per leaf so tile chrome stays clean. We
// intentionally avoid storing user-set ratios in v1 (no manual
// resize), so equal division is correct.
export function layout(tree: Node, rect: Rect): LeafRect[] {
  if (tree.kind === "leaf") {
    return [{ id: tree.id, leaf: tree, rect }];
  }
  const out: LeafRect[] = [];
  const n = tree.children.length;
  if (tree.axis === "col") {
    const childW = rect.w / n;
    tree.children.forEach((child, i) => {
      out.push(
        ...layout(child, {
          x: rect.x + i * childW,
          y: rect.y,
          w: childW,
          h: rect.h,
        }),
      );
    });
  } else {
    const childH = rect.h / n;
    tree.children.forEach((child, i) => {
      out.push(
        ...layout(child, {
          x: rect.x,
          y: rect.y + i * childH,
          w: rect.w,
          h: childH,
        }),
      );
    });
  }
  return out;
}

export function insetRect(r: Rect, inset: number): Rect {
  return {
    x: r.x + inset,
    y: r.y + inset,
    w: Math.max(0, r.w - inset * 2),
    h: Math.max(0, r.h - inset * 2),
  };
}
