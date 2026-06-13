// Store — data-access + cache layer, mirroring dialog-diagnose's
// `DiagnoseStore`. It loads tree nodes from the `TreeLoader` on demand,
// caches them by hash, tracks child→parent parentage as it goes, and
// exposes nodes as `Loadable` so views can render a pending state.
//
// This is the "model" in the data sense: the shared, lazily-populated
// picture of the tree that every view and the navigation state read.

import { type Loadable, Pending, Resolved } from "./promise.js";
import type { NodeHash, TreeEntry, TreeLoader, TreeNode } from "./types.js";

type Listener = () => void;

export class Store {
  #loader: TreeLoader;
  #root: NodeHash | null = null;
  #nodes = new Map<NodeHash, TreeNode>();
  /** Loaded child-hash lists, by index-node hash. */
  #childrenOf = new Map<NodeHash, NodeHash[]>();
  /** child hash → parent hash, filled as index nodes load. */
  #parentOf = new Map<NodeHash, NodeHash>();
  /** Memoized in-flight loads, so concurrent reads share one request. */
  #loadingNode = new Set<NodeHash>();
  #loadingChildren = new Set<NodeHash>();
  #entries = new Map<NodeHash, Promise<TreeEntry[]>>();
  #maxSize = 1;
  #listeners = new Set<Listener>();

  constructor(loader: TreeLoader) {
    this.#loader = loader;
  }

  /** Notify views that cached data changed (a load resolved). */
  subscribe(fn: Listener): () => void {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  }
  #emit(): void {
    for (const fn of this.#listeners) fn();
  }

  get rootHash(): NodeHash | null {
    return this.#root;
  }
  /** Largest node byte size seen — for size-relative scaling. */
  get maxSize(): number {
    return this.#maxSize;
  }

  /** Load the root, clearing all caches. */
  async init(): Promise<NodeHash | null> {
    this.#nodes.clear();
    this.#childrenOf.clear();
    this.#parentOf.clear();
    this.#entries.clear();
    this.#maxSize = 1;
    const root = await this.#loader.root();
    this.#root = root ? (this.#put(root), root.hash) : null;
    this.#emit();
    return this.#root;
  }

  #put(node: TreeNode): void {
    this.#nodes.set(node.hash, node);
    this.#maxSize = Math.max(this.#maxSize, node.size);
  }

  /** A node, if cached; triggers a background load on a miss. */
  node(hash: NodeHash): Loadable<TreeNode> {
    const cached = this.#nodes.get(hash);
    if (cached) return Resolved(cached);
    if (!this.#loadingNode.has(hash)) {
      this.#loadingNode.add(hash);
      // A node we don't have cached can only be reached as a child; if
      // its parent hasn't loaded children yet there's no standalone
      // node fetch, so this is a best-effort no-op for now. (The tree
      // is populated parent-first via `children`.)
      this.#loadingNode.delete(hash);
    }
    return Pending;
  }

  parentOf(hash: NodeHash): NodeHash | undefined {
    return this.#parentOf.get(hash);
  }

  /** Cached child hashes of an index node, or `Pending` (triggers a load). */
  children(hash: NodeHash): Loadable<NodeHash[]> {
    const cached = this.#childrenOf.get(hash);
    if (cached) return Resolved(cached);
    void this.#loadChildren(hash);
    return Pending;
  }

  async #loadChildren(hash: NodeHash): Promise<void> {
    if (this.#childrenOf.has(hash) || this.#loadingChildren.has(hash)) return;
    const node = this.#nodes.get(hash);
    if (!node || node.kind !== "index") return;
    this.#loadingChildren.add(hash);
    try {
      const kids = await this.#loader.children(hash);
      const hashes: NodeHash[] = [];
      for (const kid of kids) {
        this.#put(kid);
        this.#parentOf.set(kid.hash, hash);
        hashes.push(kid.hash);
      }
      this.#childrenOf.set(hash, hashes);
      this.#emit();
    } finally {
      this.#loadingChildren.delete(hash);
    }
  }

  /** Entries of a segment, memoized. */
  entries(hash: NodeHash): Promise<TreeEntry[]> {
    let p = this.#entries.get(hash);
    if (!p) {
      p = this.#loader.entries(hash);
      this.#entries.set(hash, p);
    }
    return p;
  }
}
