// <dialog-node-inspector> — detail pane for the selected tree node,
// mirroring dialog-diagnose's NodeInspector. Shows the node's hash,
// size, count, and its upper-bound key rendered as tag-colored raw byte
// segments (no labels — the colors are the segmentation, and the byte
// layout reorders by the key's tag, exactly as diagnose does). For a
// segment node, it lists the contained entries as a table (Entity ·
// Attribute · Value · State), like diagnose's FactTable; clicking a row
// reveals that entry's key as colored byte segments.
//
// Bind it to the same Store + TreeState as the outline; it redraws when
// the selection changes.

import { KEY_ROW_STYLE, renderKeyRow } from "./key-row.js";
import { isResolved } from "./promise.js";
import type { Store } from "./store.js";
import type { TreeState } from "./tree-state.js";
import type { NodeHash, TreeEntry, TreeNode } from "./types.js";

function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function shortHash(h: string, n = 16): string {
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
  h2 { margin: 0 0 var(--wa-space-s, 8px); font-size: var(--wa-font-size-xs, 12px);
       font-weight: var(--wa-font-weight-semibold, 600); color: var(--wa-color-brand-60);
       text-transform: uppercase; letter-spacing: 0.04em; }
  .kv { display: flex; gap: var(--wa-space-s, 8px); margin: 2px 0; }
  .kv .k { color: var(--wa-color-text-quiet); min-width: 64px; }
  .sizebar { display: inline-block; height: 7px; background: var(--wa-color-teal-60);
    border-radius: var(--wa-border-radius-s, 2px); vertical-align: middle; margin-left: 6px; min-width: 2px; }
  .keybytes { margin: 4px 0 6px; line-height: 1.7; }
  ${KEY_ROW_STYLE}
  .entries { margin-top: var(--wa-space-m, 12px); }
  table { width: 100%; border-collapse: collapse; font-size: var(--wa-font-size-xs, 12px); }
  th { text-align: left; color: var(--wa-color-text-quiet); font-weight: var(--wa-font-weight-semibold, 600);
       padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-normal); }
  td { padding: 3px 8px 3px 0; border-bottom: 1px solid var(--wa-color-border-quiet); vertical-align: top; }
  tr.entry { cursor: pointer; }
  tr.entry:hover { background: var(--wa-color-surface-raised, rgba(255,255,255,0.04)); }
  tr.entry.removed td { color: var(--wa-color-text-quiet); }
  .col-attr { color: var(--wa-color-yellow-60); }
  .col-ent { color: var(--wa-color-teal-60); }
  .col-type { color: var(--wa-color-blue-60); }
  tr.keyrow td { padding: 0 0 6px; border-bottom: 1px solid var(--wa-color-border-quiet); }
  .status, .error { color: var(--wa-color-text-quiet); font-style: italic; padding: 4px 0; }
  .error { color: var(--wa-color-red-60); }
  .empty { color: var(--wa-color-text-quiet); font-style: italic; }
</style>
`;

export class DialogNodeInspector extends HTMLElement {
  #root: ShadowRoot;
  #store: Store | null = null;
  #state: TreeState | null = null;
  #unsub: Array<() => void> = [];
  /** Token to drop stale async entry renders. */
  #gen = 0;

  constructor() {
    super();
    this.#root = this.attachShadow({ mode: "open" });
    this.#root.innerHTML = STYLE + `<div class="body"></div>`;
  }

  bind(store: Store, state: TreeState): void {
    this.#teardown();
    this.#store = store;
    this.#state = state;
    this.#unsub.push(state.subscribe(() => this.#render()));
    this.#unsub.push(store.subscribe(() => this.#render()));
    this.#render();
  }

  disconnectedCallback(): void {
    this.#teardown();
  }

  #teardown(): void {
    this.#unsub.forEach((fn) => fn());
    this.#unsub = [];
  }

  get #body(): HTMLElement {
    return this.#root.querySelector(".body")!;
  }

  #render(): void {
    const body = this.#body;
    body.textContent = "";
    const hash = this.#state?.selected;
    if (!hash) {
      this.#span(body, "empty", "no node selected");
      return;
    }
    const node = this.#nodeOf(hash);
    if (!node) {
      this.#span(body, "status", "loading…");
      return;
    }

    const h2 = document.createElement("h2");
    h2.textContent = node.kind === "index" ? "Index node" : "Segment node";
    body.appendChild(h2);

    body.appendChild(this.#kv("hash", shortHash(hash)));
    body.appendChild(this.#sizeKv(node.size));
    body.appendChild(
      this.#kv(node.kind === "index" ? "children" : "entries", String(node.count)),
    );
    body.appendChild(
      this.#kv("storage", node.cached === false ? "remote (fetched on demand)" : "local"),
    );

    if (node.bound) {
      body.appendChild(this.#kv("upper key", ""));
      body.appendChild(this.#keyBytes(node.bound));
    }

    if (node.kind === "segment") void this.#renderEntries(body, hash, ++this.#gen);
  }

  #kv(k: string, v: string): HTMLElement {
    const row = document.createElement("div");
    row.className = "kv";
    const kk = document.createElement("span");
    kk.className = "k";
    kk.textContent = k;
    const vv = document.createElement("span");
    vv.textContent = v;
    row.append(kk, vv);
    return row;
  }

  #sizeKv(size: number): HTMLElement {
    const row = this.#kv("size", bytes(size));
    const bar = document.createElement("span");
    bar.className = "sizebar";
    const max = this.#store?.maxSize ?? size;
    bar.style.width = `${Math.max(2, Math.round(80 * (size / max)))}px`;
    row.appendChild(bar);
    return row;
  }

  /** The key as color-coded, tooltipped component segments. */
  #keyBytes(keyStr: NodeHash): HTMLElement {
    const box = document.createElement("div");
    box.className = "keybytes";
    box.appendChild(renderKeyRow(keyStr, null));
    return box;
  }

  /** A table of the segment's entries, diagnose-FactTable style. */
  async #renderEntries(parent: HTMLElement, hash: NodeHash, gen: number): Promise<void> {
    if (!this.#store) return;
    const box = document.createElement("div");
    box.className = "entries";
    this.#span(box, "status", "loading entries…");
    parent.appendChild(box);
    try {
      const entries = await this.#store.entries(hash);
      if (gen !== this.#gen) return; // selection moved on
      box.textContent = "";
      this.#span(box, "k", `${entries.length} entries`);
      box.appendChild(this.#entryTable(entries));
    } catch (err) {
      if (gen !== this.#gen) return;
      box.textContent = "";
      this.#span(box, "error", err instanceof Error ? err.message : String(err));
    }
  }

  #entryTable(entries: TreeEntry[]): HTMLElement {
    const table = document.createElement("table");
    const thead = document.createElement("thead");
    const hr = document.createElement("tr");
    for (const h of ["Entity", "Attribute", "Type", "Value"]) {
      const th = document.createElement("th");
      th.textContent = h;
      hr.appendChild(th);
    }
    thead.appendChild(hr);
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    for (const entry of entries) {
      const tr = document.createElement("tr");
      tr.className = "entry" + (entry.state === "removed" ? " removed" : "");

      const cells: Array<[string, string]> = [
        ["col-ent", entry.entity ? shortHash(entry.entity, 12) : ""],
        ["col-attr", entry.attribute ?? ""],
        ["col-type", entry.type ?? ""],
        ["", entry.state === "removed" ? "(retracted)" : entry.value ?? ""],
      ];
      for (const [cls, text] of cells) {
        const td = document.createElement("td");
        if (cls) td.className = cls;
        td.textContent = text;
        tr.appendChild(td);
      }

      // Click a row to reveal that entry's key as colored byte segments.
      const keyTr = document.createElement("tr");
      keyTr.className = "keyrow";
      keyTr.hidden = true;
      const keyTd = document.createElement("td");
      keyTd.colSpan = 4;
      keyTd.appendChild(this.#keyBytes(entry.key));
      keyTr.appendChild(keyTd);

      tr.addEventListener("click", () => {
        keyTr.hidden = !keyTr.hidden;
      });

      tbody.append(tr, keyTr);
    }
    table.appendChild(tbody);
    return table;
  }

  #span(parent: HTMLElement, cls: string, text: string): void {
    const el = document.createElement("div");
    el.className = cls;
    el.textContent = text;
    parent.appendChild(el);
  }

  #nodeOf(hash: NodeHash): TreeNode | undefined {
    const p = this.#store?.node(hash);
    return p && isResolved(p) ? p.value : undefined;
  }
}

export function defineNodeInspector(tag = "dialog-node-inspector"): void {
  if (!customElements.get(tag)) customElements.define(tag, DialogNodeInspector);
}
