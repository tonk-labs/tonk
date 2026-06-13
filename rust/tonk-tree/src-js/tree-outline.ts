// <dialog-tree-outline> — the navigable tree outline, built on Web
// Awesome's <wa-tree>/<wa-tree-item>. wa-tree gives expand/collapse,
// keyboard navigation, selection, and lazy loading for free; we only
// supply each item's row content (hash · kind · size bar) and load
// children on demand from the shared store.
//
// Bind it to a Store + TreeState (the shared model). Selecting an item
// updates the TreeState, so a paired <dialog-node-inspector> reacts.
//
// Requires Web Awesome's wa-tree to be registered on the page (the host
// app loads it; the loader auto-registers any <wa-*> used in the DOM).

import { KEY_ROW_STYLE, renderKeyRow } from "./key-row.js";
import { isResolved } from "./promise.js";
import type { Store } from "./store.js";
import type { TreeState } from "./tree-state.js";
import type { NodeHash, TreeNode } from "./types.js";

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function shortHash(h: string, n = 8): string {
  const v = h.startsWith("#") ? h.slice(1) : h;
  return v.slice(0, n);
}

const STYLE = `
<style>
  :host {
    display: block;
    font-family: var(--wa-font-family-code, ui-monospace, monospace);
    font-size: var(--wa-font-size-s, 13px);
    color: var(--wa-color-text-normal);
  }
  .row { display: inline-flex; align-items: center; gap: var(--wa-space-s, 8px); width: 100%; }
  .keystr { white-space: nowrap; }
  ${KEY_ROW_STYLE}
  .kindicon { flex: none; }
  .kindicon.index { color: var(--wa-color-blue-60); }
  .kindicon.segment { color: var(--wa-color-green-60); }
  .count { color: var(--wa-color-text-quiet); font-size: var(--wa-font-size-xs, 11px); flex: none; }
  .sizewrap { display: inline-flex; align-items: center; gap: var(--wa-space-xs, 6px); margin-left: auto; flex: none; }
  .sizebar { height: 7px; background: var(--wa-color-teal-60); border-radius: var(--wa-border-radius-s, 2px); min-width: 2px; }
  .sizenum { color: var(--wa-color-text-quiet); font-size: var(--wa-font-size-xs, 11px); width: 56px; text-align: right; }
  .status { color: var(--wa-color-text-quiet); font-style: italic; }
  .row.remote { opacity: 0.5; }
  .row.remote .sizebar { background: var(--wa-color-neutral-fill-loud, #666); }
  .remote-icon { color: var(--wa-color-indigo-60); flex: none; }
</style>
`;

export class DialogTreeOutline extends HTMLElement {
  #root: ShadowRoot;
  #store: Store | null = null;
  #state: TreeState | null = null;
  #unsub: Array<() => void> = [];
  /** Whether an item element for a hash has been built. */
  #items = new Map<NodeHash, HTMLElement>();

  constructor() {
    super();
    this.#root = this.attachShadow({ mode: "open" });
    this.#root.innerHTML = STYLE + `<wa-tree selection="single"></wa-tree>`;
    this.#tree.addEventListener("wa-selection-change", (e) => this.#onSelect(e as CustomEvent));
  }

  /** Bind the shared store + navigation state. */
  bind(store: Store, state: TreeState): void {
    this.#teardown();
    this.#store = store;
    this.#state = state;
    this.#unsub.push(store.subscribe(() => this.#onStore()));
    this.#unsub.push(state.subscribe(() => this.#syncSelection()));
    this.#render();
  }

  /**
   * Store changed. If we have no root row yet but the store now has a
   * root (e.g. `init()` just resolved), do a full render; otherwise just
   * refresh selection / placeholder rows.
   */
  #onStore(): void {
    const needsRoot = this.#items.size === 0 && !!this.#store?.rootHash;
    if (needsRoot) this.#render();
    else this.#syncSelection();
  }

  disconnectedCallback(): void {
    this.#teardown();
  }

  #teardown(): void {
    this.#unsub.forEach((fn) => fn());
    this.#unsub = [];
    this.#items.clear();
  }

  get #tree(): HTMLElement {
    return this.#root.querySelector("wa-tree")!;
  }

  #render(): void {
    const tree = this.#tree;
    tree.textContent = "";
    this.#items.clear();
    const root = this.#store?.rootHash;
    if (!root) {
      const s = document.createElement("div");
      s.className = "status";
      s.textContent = this.#store ? "empty tree" : "not bound";
      tree.appendChild(s);
      return;
    }
    tree.appendChild(this.#item(root, null));
  }

  /** Build a <wa-tree-item> for a node, lazy if it's an expandable index. */
  #item(hash: NodeHash, parentHash: NodeHash | null): HTMLElement {
    const item = document.createElement("wa-tree-item");
    item.dataset.hash = hash;
    this.#items.set(hash, item);

    const node = this.#nodeOf(hash);
    item.appendChild(this.#row(hash, node, parentHash));

    if (node && node.kind === "index" && node.count > 0) {
      // wa-tree-item lazy: fire wa-lazy-load on first expand.
      (item as HTMLElement & { lazy: boolean }).lazy = true;
      item.addEventListener(
        "wa-lazy-load",
        (e) => {
          e.stopPropagation();
          void this.#loadInto(hash, item);
        },
        { once: true },
      );
    }
    return item;
  }

  /** Resolve children and append them as child items. */
  async #loadInto(hash: NodeHash, item: HTMLElement): Promise<void> {
    if (!this.#store) return;
    // Kick the load; wait until the store has it resolved.
    let kids = this.#store.children(hash);
    if (!isResolved(kids)) {
      await new Promise<void>((resolve) => {
        const off = this.#store!.subscribe(() => {
          kids = this.#store!.children(hash);
          if (isResolved(kids)) {
            off();
            resolve();
          }
        });
      });
    }
    if (isResolved(kids)) {
      for (const child of kids.value) item.appendChild(this.#item(child, hash));
    }
    (item as HTMLElement & { lazy: boolean }).lazy = false;
  }

  #row(hash: NodeHash, node: TreeNode | undefined, parentHash: NodeHash | null): HTMLElement {
    const row = document.createElement("span");
    row.className = "row";
    // A node not present in local storage (would fetch from remote) is
    // dimmed and flagged — at-a-glance "what's replicated here".
    if (node && node.cached === false) row.classList.add("remote");

    if (!node) {
      const s = document.createElement("span");
      s.className = "status";
      s.textContent = `${shortHash(hash)} · loading…`;
      row.appendChild(s);
      return row;
    }

    // An icon on the left distinguishes the node type at a glance: a
    // tree-of-folders for an index node (child links), a list for a
    // segment (entries).
    const icon = document.createElement("wa-icon");
    icon.className = `kindicon ${node.kind}`;
    icon.setAttribute("name", node.kind === "index" ? "folder-tree" : "list");
    icon.setAttribute(
      "label",
      node.kind === "index" ? "index node" : "segment node",
    );
    row.appendChild(icon);

    // The node's separator key, front-coded against the parent's so only
    // the bytes where this node diverges from its parent stand out.
    const keystr = document.createElement("span");
    keystr.className = "keystr";
    if (node.bound) {
      const parentNode = parentHash ? this.#nodeOf(parentHash) : undefined;
      keystr.appendChild(renderKeyRow(node.bound, parentNode?.bound ?? null));
    } else {
      keystr.textContent = shortHash(hash);
    }
    keystr.title = hash;
    row.appendChild(keystr);

    const count = document.createElement("span");
    count.className = "count";
    count.textContent =
      node.kind === "index" ? `${node.count} children` : `${node.count} entries`;
    row.appendChild(count);

    const sw = document.createElement("span");
    sw.className = "sizewrap";
    const bar = document.createElement("span");
    bar.className = "sizebar";
    const frac = Math.max(0.02, Math.min(1, node.size / (this.#store?.maxSize ?? node.size)));
    bar.style.width = `calc(120px * ${frac})`;
    const num = document.createElement("span");
    num.className = "sizenum";
    num.textContent = bytes(node.size);
    sw.append(bar, num);
    row.appendChild(sw);

    // Remote (not-locally-cached) marker.
    if (node.cached === false) {
      const cloud = document.createElement("wa-icon");
      cloud.className = "remote-icon";
      cloud.setAttribute("name", "cloud");
      cloud.setAttribute("label", "not cached locally — fetched on demand");
      row.appendChild(cloud);
    }

    return row;
  }

  #onSelect(e: CustomEvent): void {
    const selected = (e.detail as { selection?: HTMLElement[] }).selection?.[0];
    const hash = selected?.dataset.hash;
    if (hash && this.#state) this.#state.select(hash);
  }

  /** Reflect external selection / newly-loaded node rows. */
  #syncSelection(): void {
    if (!this.#state) return;
    const sel = this.#state.selected;
    for (const [hash, item] of this.#items) {
      (item as HTMLElement & { selected: boolean }).selected = hash === sel;
      // Refresh a row that was a placeholder when first built.
      const node = this.#nodeOf(hash);
      if (node && item.querySelector(".status")) {
        const old = item.querySelector(".row");
        const parent = this.#store?.parentOf(hash) ?? null;
        if (old) old.replaceWith(this.#row(hash, node, parent));
      }
    }
  }

  #nodeOf(hash: NodeHash): TreeNode | undefined {
    const p = this.#store?.node(hash);
    return p && isResolved(p) ? p.value : undefined;
  }
}

export function defineTreeOutline(tag = "dialog-tree-outline"): void {
  if (!customElements.get(tag)) customElements.define(tag, DialogTreeOutline);
}
