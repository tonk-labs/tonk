// Data shapes the component renders, and the loader it is driven by.
//
// These mirror the `tree/*` formula conclusions the worker returns
// (see plan/tree-inspector.md), but the component does not know or care
// that a worker produced them — an embedder supplies a `TreeLoader`
// that returns these shapes from wherever it likes.

/** A node hash, the `#<base58>` string the worker emits. */
export type NodeHash = string;

/** Whether a node holds child links (index) or entries (segment). */
export type NodeKind = "index" | "segment";

/** One node's fields — the `tree/node` / `tree/child` shape. */
export interface TreeNode {
  /** This node's hash. */
  hash: NodeHash;
  kind: NodeKind;
  /** Byte size of the node's stored block. */
  size: number;
  /** Children (index) or entries (segment). */
  count: number;
  /** The node's upper-bound key, `#<base58>` of its raw 162 bytes. */
  bound?: NodeHash;
  /** Sibling position, when reached as a child. */
  at?: number;
  /**
   * Whether this node's block is present in *local* storage. False
   * means it would be fetched from a remote on access — surfaced so the
   * inspector can show what is replicated here vs. not. Absent ⇒ treated
   * as cached (the loader doesn't distinguish).
   */
  cached?: boolean;
}

/** One entry in a segment node — the `tree/entry` shape. */
export interface TreeEntry {
  /** The entry's composite key, `#<base58>` of its 162 bytes. */
  key: NodeHash;
  at: number;
  state: "added" | "removed";
  entity?: string;
  attribute?: string;
  /** Value type name (e.g. "Text", "Boolean"), when surfaced. */
  type?: string;
  /** Decoded value, when the loader surfaces it. */
  value?: string;
}

/**
 * How the component reads the tree. The embedder implements this —
 * e.g. by issuing `tree/*` formula queries against a worker — and the
 * component calls it lazily as the user navigates.
 */
export interface TreeLoader {
  /** The root node of the tree, or null for an empty tree. */
  root(): Promise<TreeNode | null>;
  /** Children of an index node (one per child). */
  children(hash: NodeHash): Promise<TreeNode[]>;
  /** Entries of a segment node (one per entry). */
  entries(hash: NodeHash): Promise<TreeEntry[]>;
}
