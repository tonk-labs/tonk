// Document mode: the workbook is ONE automerge document the host owns,
// and this element holds no automerge.
//
// The grid already speaks a claims dialect: rows in (`applyRows`), one
// typed event per mutation out. Document mode reuses both seams instead
// of adding a third path through the grid. A workbook snapshot from the
// host is turned into the same row shapes the claims views render, and
// each typed event is turned into a path edit (`put` / `remove`) on the
// document. The ids the rows carry are synthetic — `<sheet>!<A1>` — so
// an event that names a row can be mapped back to its path with no
// lookup table.
//
// Every edit is a whole-value write at an address key, so two people who
// fill one empty cell write the SAME key: there is one cell, automerge
// keeps the other value as a conflict, and the claims-era defect of two
// cell entities at one address cannot occur.
//
// Pure logic; the node tests cover it. The transport is the same
// `tonk-document` event `<tonk-prose>` uses.

import type { CellRow, ColumnRow, RowSizeRow, SheetRow } from "./grid/claims";

/** One sheet of the workbook, as the host serializes it. */
export interface SheetSnapshot {
  id: string;
  name: string;
  order: string;
  cells: Record<string, string>;
  styles: Record<string, string>;
  widths: Record<string, number>;
  heights: Record<string, number>;
  /** Addresses whose cell holds concurrent values. */
  conflicts: string[];
}

/** The workbook at some heads. */
export interface TableSnapshot {
  sheets: SheetSnapshot[];
}

/** A wire edit of a table document. */
export type TableEdit =
  | { edit: "put"; path: string; value: string | number }
  | { edit: "remove"; path: string };

/** The row shapes `applyRows` takes. */
export interface Rows {
  sheets: SheetRow[];
  cells: CellRow[];
  columns: ColumnRow[];
  rowSizes: RowSizeRow[];
}

const cellId = (sheet: string, at: string) => `${sheet}!${at}`;
const columnId = (sheet: string, at: string) => `${sheet}!col:${at}`;
const rowId = (sheet: string, at: string) => `${sheet}!row:${at}`;

/** Split a synthetic row id back into its sheet and address. */
function parseId(id: unknown, marker: "" | "col:" | "row:"): { sheet: string; at: string } | null {
  if (typeof id !== "string") return null;
  const bang = id.indexOf("!");
  if (bang <= 0) return null;
  const rest = id.slice(bang + 1);
  if (!rest.startsWith(marker)) return null;
  const at = rest.slice(marker.length);
  return at === "" ? null : { sheet: id.slice(0, bang), at };
}

/** A workbook snapshot as the rows the grid reconciles against. */
export function rowsOf(subject: string, table: TableSnapshot): Rows {
  const rows: Rows = { sheets: [], cells: [], columns: [], rowSizes: [] };
  for (const sheet of table.sheets) {
    rows.sheets.push({ id: sheet.id, table: subject, name: sheet.name, order: sheet.order });
    for (const [at, content] of Object.entries(sheet.cells)) {
      rows.cells.push({
        id: cellId(sheet.id, at),
        sheet: sheet.id,
        at,
        content,
        style: sheet.styles[at] ?? "",
      });
    }
    for (const [at, width] of Object.entries(sheet.widths)) {
      rows.columns.push({ id: columnId(sheet.id, at), sheet: sheet.id, at, width: String(width) });
    }
    for (const [at, height] of Object.entries(sheet.heights)) {
      rows.rowSizes.push({ id: rowId(sheet.id, at), sheet: sheet.id, at, height: String(height) });
    }
  }
  return rows;
}

/** Every conflicted cell of a snapshot, as `<sheet>!<A1>` ids. */
export function conflictsOf(table: TableSnapshot): string[] {
  return table.sheets.flatMap((sheet) => sheet.conflicts.map((at) => cellId(sheet.id, at)));
}

/** The document edits one grid mutation event stands for. `mint` makes
 *  the id of a new sheet. An event document mode does not know maps to
 *  nothing. */
export function editsOf(
  type: string,
  detail: Record<string, unknown>,
  mint: () => string,
): TableEdit[] {
  const text = (key: string) => (typeof detail[key] === "string" ? (detail[key] as string) : null);
  const size = (key: string) => {
    const n = Number(detail[key]);
    return Number.isFinite(n) && n > 0 ? n : null;
  };
  switch (type) {
    case "createsheet": {
      const id = mint();
      return [
        { edit: "put", path: `sheets/${id}/name`, value: text("sheetName") ?? "" },
        { edit: "put", path: `sheets/${id}/order`, value: text("sheetOrder") ?? "" },
      ];
    }
    case "renamesheet": {
      const sheet = text("renameSheet");
      return sheet === null
        ? []
        : [{ edit: "put", path: `sheets/${sheet}/name`, value: text("renameName") ?? "" }];
    }
    case "createcell": {
      const sheet = text("cellSheet");
      const at = text("cellAt");
      return sheet === null || at === null
        ? []
        : [{ edit: "put", path: `sheets/${sheet}/cells/${at}`, value: text("cellContent") ?? "" }];
    }
    case "editcell": {
      const cell = parseId(detail.editCell, "");
      return cell === null
        ? []
        : [{ edit: "put", path: `sheets/${cell.sheet}/cells/${cell.at}`, value: text("editContent") ?? "" }];
    }
    case "clearcell": {
      const cell = parseId(detail.clearCell, "");
      return cell === null ? [] : [{ edit: "remove", path: `sheets/${cell.sheet}/cells/${cell.at}` }];
    }
    case "createcolumn": {
      const sheet = text("columnSheet");
      const at = text("columnAt");
      const width = size("columnWidth");
      return sheet === null || at === null || width === null
        ? []
        : [{ edit: "put", path: `sheets/${sheet}/widths/${at}`, value: width }];
    }
    case "resizecolumn": {
      const column = parseId(detail.resizeColumn, "col:");
      const width = size("resizeWidth");
      return column === null || width === null
        ? []
        : [{ edit: "put", path: `sheets/${column.sheet}/widths/${column.at}`, value: width }];
    }
    case "createrow": {
      const sheet = text("rowSheet");
      const at = text("rowAt");
      const height = size("rowHeight");
      return sheet === null || at === null || height === null
        ? []
        : [{ edit: "put", path: `sheets/${sheet}/heights/${at}`, value: height }];
    }
    case "resizerow": {
      const row = parseId(detail.resizeRow, "row:");
      const height = size("resizeHeight");
      return row === null || height === null
        ? []
        : [{ edit: "put", path: `sheets/${row.sheet}/heights/${row.at}`, value: height }];
    }
    default:
      return [];
  }
}

/** What the host answers. */
export interface Reply {
  heads: string[];
  local?: string[];
  table: TableSnapshot;
  /** The format rule: the workbook is in a format newer than this app
   *  knows. It can be shown and must not be edited. */
  readonly?: boolean;
}

export interface Transport {
  read(): Promise<Reply>;
  write(heads: string[], edits: TableEdit[]): Promise<Reply>;
}

/** Keeps a workbook element and its document in step. Edits queue and
 *  leave as ONE write, so one gesture — a paste, a fill, a row insert —
 *  is one automerge change that reaches other replicas completely or
 *  not at all. Table edits are absolute writes at a key, so unlike text
 *  the merged reply can always be applied: the grid's pending ledger
 *  keeps cells that were edited during the round trip. */
export class TableSession {
  readonly #transport: Transport;
  readonly #apply: (table: TableSnapshot) => void;
  #known: string[] = [];
  #queue: TableEdit[] = [];
  #inFlight = false;
  #opened = false;
  #closed = false;
  /** Pinned to a past version: opened once, then neither written nor
   *  polled. */
  readonly #pinned: boolean;
  readonly #onOpen: (() => void) | undefined;
  /** The host said the workbook is in a newer format. */
  #newer = false;

  constructor(
    transport: Transport,
    apply: (table: TableSnapshot) => void,
    options: { pinned?: boolean; onOpen?: () => void } = {},
  ) {
    this.#transport = transport;
    this.#apply = apply;
    this.#pinned = options.pinned === true;
    this.#onOpen = options.onOpen;
  }

  get opened(): boolean {
    return this.#opened;
  }

  /** Whether the workbook takes no edits from this element: a past
   *  version, or a format newer than this app knows. */
  get readonly(): boolean {
    return this.#pinned || this.#newer;
  }

  get pinned(): boolean {
    return this.#pinned;
  }

  get heads(): readonly string[] {
    return this.#known;
  }

  async open(): Promise<void> {
    const reply = await this.#transport.read();
    if (this.#closed) return;
    this.#known = reply.heads;
    this.#opened = true;
    this.#newer = reply.readonly === true;
    if (this.#newer) this.#queue = [];
    this.#apply(reply.table);
    this.#onOpen?.();
    if (this.#queue.length > 0) await this.flush();
  }

  /** Queue edits. The caller flushes once the gesture settled. */
  push(edits: TableEdit[]): void {
    if (this.readonly) return;
    this.#queue.push(...edits);
  }

  get pending(): number {
    return this.#queue.length;
  }

  async flush(): Promise<void> {
    if (this.#closed || !this.#opened || this.#inFlight || this.#queue.length === 0) return;
    const edits = this.#queue;
    this.#queue = [];
    this.#inFlight = true;
    try {
      const reply = await this.#transport.write(this.#known, edits);
      if (this.#closed) return;
      this.#known = reply.heads;
      this.#apply(reply.table);
    } catch (error) {
      // Keep the edits: the next flush retries them, in order.
      this.#queue = [...edits, ...this.#queue];
      throw error;
    } finally {
      this.#inFlight = false;
    }
    if (this.#queue.length > 0) await this.flush();
  }

  /** Look for changes made elsewhere. */
  async poll(): Promise<void> {
    if (this.#closed) return;
    // The first read failed — the heads can arrive before their bytes.
    // Try again; until it works the grid shows nothing and takes no edits.
    if (!this.#opened) return this.open();
    if (this.#pinned) return;
    if (this.#closed || !this.#opened || this.#inFlight || this.#queue.length > 0) return;
    const reply = await this.#transport.read();
    if (this.#closed || this.#inFlight || this.#queue.length > 0) return;
    if (sameHeads(reply.heads, this.#known)) return;
    this.#known = reply.heads;
    this.#apply(reply.table);
  }

  close(): void {
    this.#closed = true;
  }
}

/** The heads an `at` attribute names: change hashes separated by spaces. */
export function parseHeads(text: string | null): string[] {
  return (text ?? "").split(/\s+/).filter((head) => head !== "");
}

export function sameHeads(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const left = [...a].sort();
  const right = [...b].sort();
  return left.every((head, index) => head === right[index]);
}

/** Reach the host through a bubbling `tonk-document` event. The host
 *  (or, in a sealed guest, its relay) resolves the repository and the
 *  branch from the element's routing context. */
export function eventTransport(
  element: HTMLElement,
  entity: string,
  format: string,
  at: string[] = [],
): Transport {
  const call = async (write?: { heads: string[]; edits: TableEdit[] }): Promise<Reply> => {
    const detail: Record<string, unknown> = { entity, format };
    if (write) detail.write = { ...write, format };
    // A past version: the host reads at these heads instead of the branch's.
    else if (at.length > 0) detail.heads = at;
    const event = new CustomEvent("tonk-document", {
      detail,
      bubbles: true,
      composed: true,
      cancelable: true,
    });
    element.dispatchEvent(event);
    if (!event.defaultPrevented || !(detail.result instanceof Promise)) {
      throw new Error("tonk-document: no host answered");
    }
    const body = (await detail.result) as Record<string, unknown>;
    return {
      heads: (body.heads as string[]) ?? [],
      local: body.local as string[] | undefined,
      table: (body.table as TableSnapshot) ?? { sheets: [] },
      readonly: body.readonly === true,
    };
  };
  return {
    read: () => call(),
    write: (heads, edits) => call({ heads, edits }),
  };
}
