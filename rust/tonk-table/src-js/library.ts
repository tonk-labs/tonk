// The core of the library's `<table-editor>`
// (rust/tonk-core/assets/library/table.yaml).
//
// `<table-editor>` is branch data, so it has no URL to import chunks
// from: `scripts/build.mjs` builds this module into one expression, the
// element's `core` method, which evaluates to these exports. The element's
// methods (`table-editor/*.js` beside the YAML) are its custom-element
// class; everything they share that is not lifecycle lives here, typed and
// tested.
//
// The IronCalc engine is not bundled here. It is the element's `engine`
// method, the wasm as base64, which `createGrid` decodes and instantiates
// once per page. Kept apart so it changes only on an IronCalc version bump,
// and the grid can change without rewriting megabytes.

import { createGrid as mountGrid } from "./grid";
import type { GridOptions, TableGrid, TableSource } from "./grid/api";
import {
  type CellRow,
  type ColumnRow,
  type RowSizeRow,
  type SheetRow,
  readCellRows,
  readColumnRows,
  readRowSizeRows,
  readSheetRows,
} from "./grid/claims";
import { type Content, WORKBOOK_TYPE, formatContent, isWorkbookType } from "./content";
import { base64ToBytes, bytesToBase64 } from "./b64";

export { Clock, formatHlc } from "./hlc";
export { parseContent } from "./content";

/** Mount a grid into `parent`. The first call also instantiates the
 *  engine from `engine()`, the wasm as base64, once per page. */
export function createGrid(
  parent: HTMLElement,
  options: GridOptions,
  engine: () => string,
): Promise<TableGrid> {
  return mountGrid(parent, options, () => {
    const bytes = base64ToBytes(engine());
    if (!bytes || bytes.length === 0) throw new Error("the engine is not base64 wasm");
    return bytes;
  });
}

/** Decode a parsed content body into the grid's source form: workbook
 *  bytes when the envelope says so, CSV otherwise. A corrupt base64
 *  body degrades to an empty workbook rather than throwing: a bad
 *  store write must render an empty grid, not crash the element. */
export function toSource(content: Content): TableSource {
  if (isWorkbookType(content.contentType)) {
    const bytes = base64ToBytes(content.value);
    if (bytes && bytes.length > 0) return { kind: "workbook", bytes };
    console.warn("[table-editor] workbook body was not valid base64; starting empty");
    return { kind: "csv", csv: "" };
  }
  return { kind: "csv", csv: content.value };
}

/** The versioned envelope around `grid`'s lossless workbook bytes. */
export function envelope(grid: TableGrid, hlc: bigint): string {
  return formatContent({
    hlc,
    contentType: WORKBOOK_TYPE,
    value: bytesToBase64(grid.serialize()),
  });
}

/** The claim rows `host`'s light DOM holds for workbook `subject`. */
export function readRows(
  host: ParentNode,
  subject: string,
): { sheets: SheetRow[]; cells: CellRow[]; columns: ColumnRow[]; rowSizes: RowSizeRow[] } {
  const sheets = readSheetRows(host, subject);
  const ids = new Set(sheets.map((s) => s.id));
  return {
    sheets,
    cells: readCellRows(host, ids),
    columns: readColumnRows(host, ids),
    rowSizes: readRowSizeRows(host, ids),
  };
}

/** `Node.TEXT_NODE`, spelled out so this runs where `Node` is no global. */
const TEXT_NODE = 3;

/** The element's light-DOM DIRECT text nodes: its content channel.
 *  Deliberately not `textContent`: element children (a user `<style>`
 *  sheet, claims data rows) must never leak into the workbook content. */
export function lightContent(host: Node): string {
  let text = "";
  for (const node of Array.from(host.childNodes)) {
    if (node.nodeType === TEXT_NODE) text += node.nodeValue ?? "";
  }
  return text;
}

/** The CSS of `host`'s direct `<style>` children, adopted into the shadow
 *  root last so user rules win the cascade (restyling layer 3). */
export function userStyles(host: Element): string {
  let css = "";
  for (const child of Array.from(host.children)) {
    if (child.localName === "style") css += `${child.textContent ?? ""}\n`;
  }
  return css;
}

/** The data-row attributes whose mutations mean "the store changed a
 *  claim" in claims mode. The light-DOM observer filters on these (plus
 *  structural childList changes) so a row update triggers one debounced
 *  re-read. */
export const ROW_ATTRS = [
  "subject",
  "data-table",
  "data-name",
  "data-order",
  "data-sheet",
  "data-at",
  "data-content",
  "data-style",
  "data-width",
  "data-height",
];

/** Host stylesheet. Each `--tonk-table-*` variable is the theming
 *  contract, and its DEFAULT inherits the page's design tokens where
 *  the consumer exposes them (WebAwesome `--wa-*` tokens, the tonk
 *  app's palette), falling back to a self-contained GitHub-flavored
 *  value so the element still looks right mounted on a bare page. The
 *  grid core adds the rules for its own DOM (headers, cells, tabs) when
 *  it mounts. */
export const SHADOW_STYLESHEET = `
  :host {
    --tonk-table-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-table-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-table-font-size: var(--wa-font-size-s, 0.875rem);
    --tonk-table-radius: var(--wa-border-radius-m, 6px);

    /* Surfaces & text: inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-table-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-table-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-table-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-table-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #e5e9ed);
    --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-table-header-fg: var(--wa-color-text-quiet, #59636e);
    /* Active cell + focus: the brand accent; range fill: its quiet
       counterpart. */
    --tonk-table-accent: var(--wa-color-brand-fill-loud, #0969da);
    --tonk-table-selection: var(--wa-color-brand-fill-quiet, #0969da1a);
    --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #0969da66);
    --tonk-table-error: var(--wa-color-danger-fill-loud, #d1242f);

    display: flex;
    flex-direction: column;
    block-size: var(--tonk-table-height, 26rem);
    position: relative;
    box-sizing: border-box;
    background: var(--tonk-table-bg);
    color: var(--tonk-table-fg);
    border: 1px solid var(--tonk-table-border);
    border-radius: var(--tonk-table-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  /* Standalone dark fallback (no WebAwesome tokens present). When the
     page provides \`--wa-*\` the rules above already track its
     light/dark palette, so this only bites a bare page in dark mode. */
  @media (prefers-color-scheme: dark) {
    :host {
      --tonk-table-bg: var(--wa-color-surface-default, #0d1117);
      --tonk-table-fg: var(--wa-color-text-normal, #f0f6fc);
      --tonk-table-fg-muted: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-border: var(--wa-color-neutral-border-quiet, #3d444d);
      --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #2a3038);
      --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #151b23);
      --tonk-table-header-fg: var(--wa-color-text-quiet, #9198a1);
      --tonk-table-accent: var(--wa-color-brand-fill-loud, #1f6feb);
      --tonk-table-selection: var(--wa-color-brand-fill-quiet, #1f6feb33);
      --tonk-table-focus-ring: var(--wa-color-brand-border-normal, #1f6feb99);
      --tonk-table-error: var(--wa-color-danger-fill-loud, #f85149);
    }
  }

  :host([hidden]) { display: none; }

  :host(:focus-within) {
    border-color: var(--tonk-table-accent);
    box-shadow: 0 0 0 2px var(--tonk-table-focus-ring);
  }

  .mount {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
  .mount > .table-root { flex: 1; }
`;
