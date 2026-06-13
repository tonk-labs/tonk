// <tonk-tree> — the self-contained tree inspector element.
//
// Drop it into a tonk page and it wires itself: it finds the repository
// it lives under (a `<tonk-repository name=…>` ancestor, or an explicit
// `repo` / `branch` attribute), builds a `WorkerTreeLoader` against the
// worker's `tree/*` query formulas, and renders a `<dialog-tree-outline>`
// + `<dialog-node-inspector>` over a shared store. This is the element a
// `dialog/diagnose` view mounts.
//
// `dialog-tree-outline` / `dialog-node-inspector` stay framework-
// agnostic; this element is the thin tonk-aware shell that supplies them
// a loader from context.

import { defineNodeInspector } from "./node-inspector.js";
import { Store } from "./store.js";
import { TreeState } from "./tree-state.js";
import { defineTreeOutline } from "./tree-outline.js";
import { WorkerTreeLoader } from "./worker-loader.js";

const TEMPLATE = `
<style>
  :host { display: grid; grid-template-columns: 1.4fr 1fr; height: 100%;
    min-height: 240px; color: var(--wa-color-text-normal);
    background: var(--wa-color-surface-default, transparent); }
  .pane { overflow: auto; padding: var(--wa-space-m, 12px); }
  .left { border-right: 1px solid var(--wa-color-border-quiet); }
  .err { color: var(--wa-color-red-60); padding: var(--wa-space-m, 12px);
    font-family: var(--wa-font-family-code, monospace); }
</style>
<div class="pane left"><dialog-tree-outline></dialog-tree-outline></div>
<div class="pane right"><dialog-node-inspector></dialog-node-inspector></div>
`;

export class TonkTree extends HTMLElement {
  #root: ShadowRoot;
  #bound = false;

  static get observedAttributes(): string[] {
    return ["repo", "branch"];
  }

  constructor() {
    super();
    this.#root = this.attachShadow({ mode: "open" });
    this.#root.innerHTML = TEMPLATE;
  }

  connectedCallback(): void {
    // Defer one tick so ancestors (the <tonk-repository> wrapper) are
    // present and their `name` attribute is set.
    queueMicrotask(() => void this.#start());
  }

  attributeChangedCallback(): void {
    if (this.isConnected) void this.#start();
  }

  async #start(): Promise<void> {
    await this.#ensureWaTree();
    this.#wire();
  }

  /**
   * Web Awesome's auto-loader does not scan shadow DOM, so `<wa-tree>` /
   * `<wa-tree-item>` (which the outline renders inside its own shadow
   * root) are never registered for us. Import them explicitly from the
   * served Web Awesome dist. The base path is the `wa` attribute, or the
   * conventional `/webawesome`.
   */
  async #ensureWaTree(): Promise<void> {
    if (customElements.get("wa-tree-item")) return;
    const base = this.getAttribute("wa") || "/webawesome";
    try {
      await import(/* @vite-ignore */ `${base}/components/tree/tree.js`);
      await import(/* @vite-ignore */ `${base}/components/tree-item/tree-item.js`);
      await customElements.whenDefined("wa-tree-item");
    } catch {
      // Leave unregistered; the outline still renders rows, just flat.
    }
  }

  /** Resolve repo/branch from attributes or the <tonk-repository> ancestor. */
  #resolve(): { repo: string; branch: string } | null {
    const branch = this.getAttribute("branch") || this.#ancestorAttr("name", "tonk-branch") || "main";
    const repo = this.getAttribute("repo") || this.#ancestorAttr("name", "tonk-repository");
    return repo ? { repo, branch } : null;
  }

  /** Walk up (through shadow boundaries) for an element's attribute. */
  #ancestorAttr(attr: string, tag: string): string | null {
    let node: Node | null = this;
    while (node) {
      if (node instanceof Element && node.localName === tag) {
        const v = node.getAttribute(attr);
        if (v) return v;
      }
      const parent: Node | null =
        (node as Element).assignedSlot ??
        node.parentNode ??
        (node.getRootNode() as ShadowRoot).host ??
        null;
      node = parent === node ? null : parent;
    }
    return null;
  }

  #wire(): void {
    const ctx = this.#resolve();
    const outline = this.#root.querySelector("dialog-tree-outline") as
      | (HTMLElement & { bind(s: Store, t: TreeState): void })
      | null;
    const inspector = this.#root.querySelector("dialog-node-inspector") as
      | (HTMLElement & { bind(s: Store, t: TreeState): void })
      | null;
    if (!ctx || !outline || !inspector) {
      if (!ctx) this.#error("no repository in context (set repo= or nest under <tonk-repository>)");
      return;
    }
    this.#clearError();

    const loader = new WorkerTreeLoader({ repo: ctx.repo, branch: ctx.branch });
    const store = new Store(loader);
    const state = new TreeState(store);
    outline.bind(store, state);
    inspector.bind(store, state);
    this.#bound = true;
    void store.init().then(() => state.selectRootIfUnset());
  }

  #error(msg: string): void {
    if (this.#bound) return;
    this.#clearError();
    const el = document.createElement("div");
    el.className = "err";
    el.textContent = msg;
    this.#root.appendChild(el);
  }
  #clearError(): void {
    this.#root.querySelector(".err")?.remove();
  }
}

export function defineTonkTree(tag = "tonk-tree"): void {
  defineTreeOutline();
  defineNodeInspector();
  if (!customElements.get(tag)) customElements.define(tag, TonkTree);
}
