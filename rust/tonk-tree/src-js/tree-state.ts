// TreeState — navigation + expansion state, mirroring dialog-diagnose's
// `TreeState`. Holds the root, the selected node, and the set of
// expanded nodes, and provides depth-first navigation (next/previous)
// and expand/collapse, walking the tree through the `Store`.
//
// Views bind to one TreeState + its Store: a selection or expansion made
// in any view updates this state and notifies subscribers, so every
// view stays in sync.

import { isResolved } from "./promise.js";
import type { Store } from "./store.js";
import type { NodeHash } from "./types.js";

type Listener = () => void;

export class TreeState {
  #store: Store;
  #selected: NodeHash | null = null;
  #expanded = new Set<NodeHash>();
  #listeners = new Set<Listener>();

  constructor(store: Store) {
    this.#store = store;
  }

  subscribe(fn: Listener): () => void {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  }
  #emit(): void {
    for (const fn of this.#listeners) fn();
  }

  get store(): Store {
    return this.#store;
  }
  get selected(): NodeHash | null {
    return this.#selected;
  }
  get root(): NodeHash | null {
    return this.#store.rootHash;
  }
  isExpanded(hash: NodeHash): boolean {
    return this.#expanded.has(hash);
  }

  select(hash: NodeHash): void {
    if (this.#selected === hash) return;
    this.#selected = hash;
    this.#emit();
  }

  /** Default the selection to the root once it loads. */
  selectRootIfUnset(): void {
    if (this.#selected === null && this.root) {
      this.#selected = this.root;
      this.#emit();
    }
  }

  /** Toggle an index node's expansion (loading its children lazily). */
  toggle(hash: NodeHash): void {
    const node = this.#node(hash);
    if (!node || node.kind !== "index" || node.count === 0) return;
    if (this.#expanded.has(hash)) {
      this.#expanded.delete(hash);
    } else {
      this.#expanded.add(hash);
      // Warm the children cache; the store emits when it resolves.
      this.#store.children(hash);
    }
    this.#emit();
  }

  expand(hash: NodeHash): void {
    if (!this.#expanded.has(hash)) this.toggle(hash);
  }

  collapse(hash: NodeHash): void {
    if (this.#expanded.has(hash)) this.toggle(hash);
  }

  /** Visible nodes in depth-first display order, honoring expansion. */
  visible(): NodeHash[] {
    const out: NodeHash[] = [];
    const walk = (hash: NodeHash) => {
      out.push(hash);
      if (this.#expanded.has(hash)) {
        const kids = this.#store.children(hash);
        if (isResolved(kids)) kids.value.forEach(walk);
      }
    };
    if (this.root) walk(this.root);
    return out;
  }

  /** Move selection to the next visible node (depth-first). */
  selectNext(): void {
    const rows = this.visible();
    const i = this.#selected ? rows.indexOf(this.#selected) : -1;
    if (i < rows.length - 1) this.select(rows[i + 1]);
  }

  /** Move selection to the previous visible node. */
  selectPrevious(): void {
    const rows = this.visible();
    const i = this.#selected ? rows.indexOf(this.#selected) : -1;
    if (i > 0) this.select(rows[i - 1]);
  }

  #node(hash: NodeHash) {
    const p = this.#store.node(hash);
    return isResolved(p) ? p.value : undefined;
  }
}
