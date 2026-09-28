// The shell surface the grid core exposes for a `<tonk-table>` whose
// SHELL lives in branch notation rather than in this bundle.
//
// `library/table.yaml` declares the element — its lifecycle methods,
// its accessors, its attribute defaults — and the browser resolves it
// from the branch the first time a `<tonk-table>` is rendered. Those
// methods are arrow functions in a keyed dictionary: no module scope,
// no imports, no constructor. Anything they need that is more than a
// few lines has to reach them through the one object they can already
// get hold of, which is this module.
//
// So this file is the seam. It re-exports the pure helpers the shell
// used to import (HLC clock, content envelope, base64, claim-row
// readers) and holds the host stylesheet, which is markup the shell
// installs rather than logic it runs. Everything here is pure or
// inert; the stateful grid stays in `./index.ts`.

import { readCellRows, readColumnRows, readRowSizeRows, readSheetRows } from "./claims";
import { Clock, formatHlc } from "../hlc";
import {
  type Content,
  WORKBOOK_TYPE,
  formatContent,
  isWorkbookType,
  parseContent,
} from "../content";
import { base64ToBytes, bytesToBase64 } from "../b64";
import type { TableSource } from "./api";

/** Decode a parsed content body into the grid's source form: workbook
 *  bytes when the envelope says so, CSV otherwise. A corrupt base64
 *  body degrades to an empty workbook rather than throwing — a bad
 *  store write must render an empty grid, not crash the element. */
export function toSource(content: Content): TableSource {
  if (isWorkbookType(content.contentType)) {
    const bytes = base64ToBytes(content.value);
    if (bytes && bytes.length > 0) return { kind: "workbook", bytes };
    console.warn("[tonk-table] workbook body was not valid base64; starting empty");
    return { kind: "csv", csv: "" };
  }
  return { kind: "csv", csv: content.value };
}

/** The `<tonk-table>` host stylesheet, installed into the shadow root
 *  before the grid mounts.
 *
 *  Every value falls back to a plain one, so the element still looks
 *  right on a bare page, and reads the surrounding page's `--wa-*`
 *  tokens when they are present — custom properties inherit through
 *  the shadow boundary. The grid core adds the rules for its own DOM
 *  (headers, cells, tabs) when it mounts. */
export const hostStyles = `
  :host {
    --tonk-table-font: var(--wa-font-family-body, ui-sans-serif, -apple-system,
                       "Segoe UI", Helvetica, Arial, sans-serif);
    --tonk-table-mono: var(--wa-font-family-code, ui-monospace, SFMono-Regular,
                       Menlo, Consolas, "Liberation Mono", monospace);
    --tonk-table-font-size: var(--wa-font-size-s, 0.875rem);
    --tonk-table-radius: var(--wa-border-radius-m, 6px);

    /* Surfaces & text — inherit the page's WebAwesome tokens, GitHub
       light values as the standalone fallback. */
    --tonk-table-bg: var(--wa-color-surface-default, #ffffff);
    --tonk-table-fg: var(--wa-color-text-normal, #1f2328);
    --tonk-table-fg-muted: var(--wa-color-text-quiet, #59636e);
    --tonk-table-border: var(--wa-color-neutral-border-quiet, #d1d9e0);
    --tonk-table-grid-line: var(--wa-color-neutral-border-quiet, #e5e9ed);
    --tonk-table-header-bg: var(--wa-color-neutral-fill-quiet, #f6f8fa);
    --tonk-table-header-fg: var(--wa-color-text-quiet, #59636e);
    /* Active cell + focus → the brand accent; range fill → its quiet
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

/** Everything the branch-resident shell calls, in one object.
 *
 *  One export rather than a dozen, because the shell reaches it
 *  through a dynamic import in a notation method body: `core.shell.x`
 *  is one lookup an author can see the shape of, where a dozen
 *  top-level names would each be a separate thing to remember and to
 *  keep in step. */
export const shell = {
  Clock,
  formatHlc,
  parseContent,
  formatContent,
  isWorkbookType,
  WORKBOOK_TYPE,
  base64ToBytes,
  bytesToBase64,
  readSheetRows,
  readCellRows,
  readColumnRows,
  readRowSizeRows,
  toSource,
} as const;
