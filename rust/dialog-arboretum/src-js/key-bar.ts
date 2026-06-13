// <dialog-tree-key> — renders one composite index key as a colored
// component bar: Tag · Entity · Attribute · ValueType · ValueRef.
//
// This is the atomic visual of the inspector. Set its `.key` property
// to a decoded `TreeKey`; it draws each component as a colored cell
// with a short value and a label beneath, the way the index-key
// diagram does. Pure presentation — no data loading.

import type { TreeKey } from "./types.js";

/** Per-component colors, matching the index-key reference palette. */
const COLORS = {
  tag: "#c792ea",
  entity: "#ff8da1",
  attribute: "#ffc78e",
  valueType: "#a1c8ff",
  valueRef: "#4ecbc4",
} as const;

/** Shorten a long base58/hex component for inline display. */
function short(s: string, head = 6, tail = 4): string {
  const v = s.startsWith("#") ? s.slice(1) : s;
  return v.length > head + tail + 1 ? `${v.slice(0, head)}…${v.slice(-tail)}` : v;
}

const TEMPLATE = `
<style>
  :host { display: inline-block; font-family: ui-monospace, monospace; }
  .bar { display: flex; align-items: stretch; gap: 0; }
  .cell {
    display: flex; flex-direction: column; padding: 0 6px 2px;
    border-bottom: 3px solid currentColor;
  }
  .val { font-weight: 600; font-size: 13px; line-height: 1.4; color: #e8e8e8; }
  .label { font-size: 9px; line-height: 1.2; opacity: 0.85; letter-spacing: 0.02em; }
  .sep { align-self: center; color: #666; padding: 0 1px; }
</style>
<div class="bar" part="bar"></div>
`;

export class DialogTreeKey extends HTMLElement {
  #root: ShadowRoot;
  #key: TreeKey | null = null;

  constructor() {
    super();
    this.#root = this.attachShadow({ mode: "open" });
    this.#root.innerHTML = TEMPLATE;
  }

  /** The decoded key to render. */
  get key(): TreeKey | null {
    return this.#key;
  }
  set key(value: TreeKey | null) {
    this.#key = value;
    this.#render();
  }

  #render(): void {
    const bar = this.#root.querySelector(".bar")!;
    bar.textContent = "";
    const k = this.#key;
    if (!k) return;

    const cells: Array<[string, string, string]> = [
      [COLORS.tag, k.tag, "Tag"],
      [COLORS.entity, short(k.entity), "Entity"],
      [COLORS.attribute, short(k.attribute), "Attribute"],
      [COLORS.valueType, String(k.valueType), "Type"],
      [COLORS.valueRef, short(k.valueRef), "Value"],
    ];

    cells.forEach(([color, value, label], i) => {
      if (i > 0) {
        const sep = document.createElement("span");
        sep.className = "sep";
        sep.textContent = ".";
        bar.appendChild(sep);
      }
      const cell = document.createElement("div");
      cell.className = "cell";
      cell.style.color = color;
      const val = document.createElement("span");
      val.className = "val";
      val.textContent = value;
      const lab = document.createElement("span");
      lab.className = "label";
      lab.textContent = label;
      cell.append(val, lab);
      bar.appendChild(cell);
    });
  }
}

export function defineKeyBar(tag = "dialog-tree-key"): void {
  if (!customElements.get(tag)) {
    customElements.define(tag, DialogTreeKey);
  }
}
