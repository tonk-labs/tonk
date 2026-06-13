// tonk-tree — web components for visualizing and inspecting a
// dialog index tree, in the style of dialog-diagnose.
//
// Layers:
//   - Store / TreeState / Loadable — the shared data + navigation model.
//   - <dialog-tree-outline> / <dialog-node-inspector> — framework-
//     agnostic views over that model (bind a Store + TreeState).
//   - <tonk-tree> — the self-contained tonk-aware element: it
//     resolves the repository from context (a <tonk-repository> ancestor
//     or repo/branch attributes), builds a WorkerTreeLoader against the
//     worker's tree/* queries, and renders the two views. This is what a
//     `dialog/diagnose` view mounts.

export * from "./types.js";
export * from "./promise.js";
export { Store } from "./store.js";
export { TreeState } from "./tree-state.js";
export { WorkerTreeLoader, type WorkerLoaderOptions } from "./worker-loader.js";
export * from "./key-bytes.js";

export { DialogTreeOutline } from "./tree-outline.js";
export { DialogNodeInspector } from "./node-inspector.js";
export { TonkTree } from "./element.js";

import { defineTonkTree } from "./element.js";

/** Register all arboretum custom elements. Idempotent. */
export function define(): void {
  defineTonkTree();
}

define();
