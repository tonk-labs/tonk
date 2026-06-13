// <dialog-arboretum> — a collapsible, lazy inspector for a dialog
// index tree. Framework-agnostic: drive it by setting `.loader` to a
// `TreeLoader` (see types.ts); it walks the tree on demand as the user
// expands rows, rendering each node's kind, byte size, and child/entry
// count, and decoding leaf entries' keys into component bars.
//
// Knows nothing about tonk or the worker — the embedder supplies the
// loader (e.g. backed by `tree/*` formula queries).

import { defineKeyBar } from "./key-bar.js";
import type { TreeEntry, TreeKey, TreeLoader, TreeNode } from "./types.js";

export type { TreeEntry, TreeKey, TreeLoader, TreeNode } from "./types.js";
export { DialogTreeKey } from "./key-bar.js";

/** Format a byte count compactly (e.g. 22 KB, 150 KB). */
function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function shortHash(h: string, head = 8, tail = 4): string {
  const v = h.startsWith("#") ? h.slice(1) : h;
  return v.length > head + tail + 1 ? `${v.slice(0, head)}…${v.slice(-tail)}` : v;
}

const TEMPLATE = `
<style>
  :host {
    display: block; font-family: ui-monospace, monospace; font-size: 13px;
    color: #ddd; --row-pad: 16px; --max-bar: 120px;
  }
  .tree { display: flex; flex-direction: column; }
  .row {
    display: flex; align-items: center; gap: 8px;
    padding: 3px 6px; border-radius: 4px; cursor: default; white-space: nowrap;
  }
  .row:hover { background: rgba(255,255,255,0.05); }
  .twist {
    width: 14px; text-align: center; cursor: pointer; color: #888;
    user-select: none; flex: none;
  }
  .twist.leaf-marker { cursor: default; color: #555; }
  .hash { color: #9ad; }
  .kind { font-size: 11px; padding: 1px 5px; border-radius: 3px; }
  .kind.branch { background: #2a3b52; color: #a1c8ff; }
  .kind.leaf { background: #2a4536; color: #7fd6a0; }
  .count { color: #888; font-size: 11px; }
  .sizewrap { display: flex; align-items: center; gap: 6px; margin-left: auto; }
  .sizebar { height: 8px; background: #4ecbc4; border-radius: 2px; min-width: 2px; }
  .sizenum { color: #aaa; font-size: 11px; width: 56px; text-align: right; }
  .children { margin-left: var(--row-pad); border-left: 1px solid #2a2a2a; }
  .entry { padding: 2px 6px; }
  .entry .meta { color: #888; font-size: 11px; margin-left: 22px; }
  .status { color: #888; padding: 4px 6px; font-style: italic; }
  .error { color: #ff8da1; padding: 4px 6px; }
</style>
<div class="tree" part="tree"></div>
`;

export class DialogArboretum extends HTMLElement {
  #root: ShadowRoot;
  #loader: TreeLoader | null = null;
  /** Largest node size seen, for scaling the size bars. */
  #maxSize = 1;

  constructor() {
    super();
    this.#root = this.attachShadow({ mode: "open" });
    this.#root.innerHTML = TEMPLATE;
  }

  get loader(): TreeLoader | null {
    return this.#loader;
  }
  set loader(value: TreeLoader | null) {
    this.#loader = value;
    void this.#renderRoot();
  }

  /** Re-fetch and redraw from the root. */
  async refresh(): Promise<void> {
    await this.#renderRoot();
  }

  get #container(): HTMLElement {
    return this.#root.querySelector(".tree")!;
  }

  async #renderRoot(): Promise<void> {
    const tree = this.#container;
    tree.textContent = "";
    if (!this.#loader) {
      this.#status(tree, "no loader set");
      return;
    }
    this.#status(tree, "loading…");
    try {
      const root = await this.#loader.root();
      tree.textContent = "";
      if (!root) {
        this.#status(tree, "empty tree");
        return;
      }
      this.#maxSize = Math.max(this.#maxSize, root.size);
      tree.appendChild(this.#nodeRow(root, 0));
    } catch (err) {
      tree.textContent = "";
      this.#errorMsg(tree, err);
    }
  }

  /** A node row plus a (lazily filled) container for its children/entries. */
  #nodeRow(node: TreeNode, depth: number): HTMLElement {
    const wrap = document.createElement("div");
    const row = document.createElement("div");
    row.className = "row";

    const twist = document.createElement("span");
    twist.className = "twist";
    const expandable = node.count > 0;
    twist.textContent = expandable ? "▸" : "·";
    if (!expandable) twist.classList.add("leaf-marker");
    row.appendChild(twist);

    const hash = document.createElement("span");
    hash.className = "hash";
    hash.textContent = shortHash(node.hash);
    hash.title = node.hash;
    row.appendChild(hash);

    const kind = document.createElement("span");
    kind.className = `kind ${node.kind}`;
    kind.textContent = node.kind;
    row.appendChild(kind);

    const count = document.createElement("span");
    count.className = "count";
    count.textContent = node.kind === "branch" ? `${node.count} children` : `${node.count} entries`;
    row.appendChild(count);

    row.appendChild(this.#sizeCell(node.size));

    const childBox = document.createElement("div");
    childBox.className = "children";
    childBox.hidden = true;

    let loaded = false;
    let open = false;
    const toggle = async () => {
      if (!expandable) return;
      open = !open;
      twist.textContent = open ? "▾" : "▸";
      childBox.hidden = !open;
      if (open && !loaded) {
        loaded = true;
        await this.#fillChildren(node, childBox, depth + 1);
      }
    };
    twist.addEventListener("click", toggle);
    row.addEventListener("dblclick", toggle);

    wrap.append(row, childBox);
    return wrap;
  }

  #sizeCell(size: number): HTMLElement {
    const wrap = document.createElement("span");
    wrap.className = "sizewrap";
    const bar = document.createElement("span");
    bar.className = "sizebar";
    const frac = Math.max(0.02, Math.min(1, size / this.#maxSize));
    bar.style.width = `calc(var(--max-bar) * ${frac})`;
    const num = document.createElement("span");
    num.className = "sizenum";
    num.textContent = bytes(size);
    wrap.append(bar, num);
    return wrap;
  }

  /** Load and append a node's children (branch) or entries (leaf). */
  async #fillChildren(node: TreeNode, box: HTMLElement, depth: number): Promise<void> {
    if (!this.#loader) return;
    this.#status(box, "loading…");
    try {
      box.textContent = "";
      if (node.kind === "branch") {
        const kids = await this.#loader.children(node.hash);
        for (const kid of kids) {
          this.#maxSize = Math.max(this.#maxSize, kid.size);
        }
        for (const kid of kids) box.appendChild(this.#nodeRow(kid, depth));
      } else {
        const entries = await this.#loader.entries(node.hash);
        for (const entry of entries) box.appendChild(this.#entryRow(entry));
      }
    } catch (err) {
      box.textContent = "";
      this.#errorMsg(box, err);
    }
  }

  /** A leaf entry: its fact summary, expandable to the decoded key bar. */
  #entryRow(entry: TreeEntry): HTMLElement {
    const wrap = document.createElement("div");
    wrap.className = "entry";

    const row = document.createElement("div");
    row.className = "row";
    const twist = document.createElement("span");
    twist.className = "twist";
    twist.textContent = "▸";
    row.appendChild(twist);

    const label = document.createElement("span");
    label.textContent = entry.state === "removed"
      ? "(retracted)"
      : `${entry.attribute ?? "?"}`;
    label.style.color = entry.state === "removed" ? "#888" : "#ffc78e";
    row.appendChild(label);

    const meta = document.createElement("span");
    meta.className = "count";
    meta.textContent = entry.entity ? shortHash(entry.entity) : "";
    row.appendChild(meta);

    const keyBox = document.createElement("div");
    keyBox.className = "children";
    keyBox.hidden = true;

    let loaded = false;
    let open = false;
    const toggle = async () => {
      open = !open;
      twist.textContent = open ? "▾" : "▸";
      keyBox.hidden = !open;
      if (open && !loaded) {
        loaded = true;
        await this.#fillKey(entry.key, keyBox);
      }
    };
    twist.addEventListener("click", toggle);
    row.addEventListener("dblclick", toggle);

    wrap.append(row, keyBox);
    return wrap;
  }

  async #fillKey(key: string, box: HTMLElement): Promise<void> {
    if (!this.#loader) return;
    this.#status(box, "decoding…");
    try {
      const decoded: TreeKey = await this.#loader.decodeKey(key);
      box.textContent = "";
      const bar = document.createElement("dialog-tree-key") as HTMLElement & { key: TreeKey };
      bar.key = decoded;
      bar.style.margin = "4px 0 4px 22px";
      box.appendChild(bar);
    } catch (err) {
      box.textContent = "";
      this.#errorMsg(box, err);
    }
  }

  #status(parent: HTMLElement, text: string): void {
    const el = document.createElement("div");
    el.className = "status";
    el.textContent = text;
    parent.appendChild(el);
  }

  #errorMsg(parent: HTMLElement, err: unknown): void {
    const el = document.createElement("div");
    el.className = "error";
    el.textContent = err instanceof Error ? err.message : String(err);
    parent.appendChild(el);
  }
}

/** Register both elements. Idempotent. */
export function define(tag = "dialog-arboretum"): void {
  defineKeyBar();
  if (!customElements.get(tag)) {
    customElements.define(tag, DialogArboretum);
  }
}

define();
