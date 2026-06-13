// Data shapes the component renders, and the loader it is driven by.
//
// These mirror the `tree/*` formula conclusions the worker returns
// (see plan/tree-inspector.md), but the component does not know or
// care that a worker produced them — an embedder supplies a `TreeLoader`
// that returns these shapes from wherever it likes.

/** A node hash, the `#<base58>` string the worker emits. */
export type NodeHash = string;

/** Whether a node holds children (branch) or entries (leaf). */
export type NodeKind = "branch" | "leaf";

/** One node's scalar fields — the `tree/node` / `tree/child` shape. */
export interface TreeNode {
  /** This node's hash. */
  hash: NodeHash;
  kind: NodeKind;
  /** Byte size of the node's stored block. */
  size: number;
  /** Children (branch) or entries (leaf). */
  count: number;
  /** Sibling position, when this node was reached as a child. */
  at?: number;
}

/** One entry in a leaf — the `tree/entry` shape. */
export interface TreeEntry {
  /** The entry's composite key, `#<base58>` of its 162 bytes. */
  key: NodeHash;
  at: number;
  state: "added" | "removed";
  entity?: string;
  attribute?: string;
  valueType?: number;
}

/** A decoded composite key — the `tree/key` shape. */
export interface TreeKey {
  key: NodeHash;
  /** Index ordering: entity / attribute / value. */
  tag: string;
  entity: string;
  attribute: string;
  valueType: number;
  valueRef: string;
}

/**
 * How the component reads the tree. The embedder implements this —
 * e.g. by issuing `tree/*` formula queries against a worker — and the
 * component calls it lazily as the user expands nodes.
 */
export interface TreeLoader {
  /** The root node of the tree, or null for an empty tree. */
  root(): Promise<TreeNode | null>;
  /** Children of a branch node (one per child). */
  children(hash: NodeHash): Promise<TreeNode[]>;
  /** Entries of a leaf node (one per entry). */
  entries(hash: NodeHash): Promise<TreeEntry[]>;
  /** Decompose a composite key into its components. */
  decodeKey(key: NodeHash): Promise<TreeKey>;
}
