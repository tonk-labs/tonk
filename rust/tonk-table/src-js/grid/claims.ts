// Claims-mode plumbing, pure logic: A1-address math, hidden data-row
// parsing, fractional order keys, and snapshot diffing. The dialog
// store individuates the workbook as claims — one instance per sheet
// and per non-empty cell — and the view materializes each claim as a
// hidden row div inside the element's light DOM (the board pattern):
//
//   <div class="table-sheet-row" subject={this} data-table={table}
//        data-name={name} data-order={order}></div>
//   <div class="table-cell-row" subject={this} data-sheet={sheet}
//        data-at={at} data-content={content} data-style={style}></div>
//
// The grid reads and observes those rows (claims IN) and emits one
// typed CustomEvent per mutation (claims OUT); commands and rules in
// table.yaml do the actual asserting/retracting. DOM-free except for
// the row readers' Element inputs, so the node tests cover everything.

/** One sheet claim, read off a `.table-sheet-row`. */
export interface SheetRow {
  /** The sheet claim's entity id (`subject` attribute). */
  id: string;
  /** The owning workbook entity (`data-table`). */
  table: string;
  name: string;
  /** Lexicographic order key (`data-order`). */
  order: string;
}

/** One cell claim, read off a `.table-cell-row`. */
export interface CellRow {
  /** The cell claim's entity id (`subject` attribute). */
  id: string;
  /** The owning sheet claim's entity id (`data-sheet`). */
  sheet: string;
  /** A1-style address (`data-at`). */
  at: string;
  /** Raw cell input (`"=A1*3"`, `"23"`, …). */
  content: string;
  /** Optional style JSON (`data-style`), `""` when unstyled. */
  style: string;
}

/** One column-sizing claim, read off a `.table-column-row`. Only
 *  explicitly-sized columns have one; everything else uses the engine
 *  default. */
export interface ColumnRow {
  id: string;
  sheet: string;
  /** Column letters ("B"). */
  at: string;
  /** Width in px, as decimal text. */
  width: string;
}

/** One row-sizing claim, read off a `.table-rowsize-row`. */
export interface RowSizeRow {
  id: string;
  sheet: string;
  /** 1-based row number, as text ("2"). */
  at: string;
  /** Height in px, as decimal text. */
  height: string;
}

/** Column letters ↔ 1-based numbers, bijective base-26 (A=1, Z=26,
 *  AA=27). Engine-free so this module stays pure. */
export function columnNumber(name: string): number | null {
  if (!/^[A-Z]+$/.test(name)) return null;
  let n = 0;
  for (const ch of name) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n;
}

export function columnName(n: number): string {
  let out = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    out = String.fromCharCode(65 + rem) + out;
    n = Math.floor((n - 1) / 26);
  }
  return out;
}

/** Parse an A1-style address into 1-based coordinates, or null when
 *  malformed. Bad addresses come from hand-written claims — an agent
 *  asserting `--at "b2"` or `--at "2B"` must not crash the grid. */
export function parseAddress(at: string): { row: number; column: number } | null {
  const m = /^([A-Z]+)([1-9][0-9]*)$/.exec(at);
  if (!m) return null;
  const column = columnNumber(m[1]);
  if (column === null) return null;
  return { row: Number(m[2]), column };
}

export function formatAddress(row: number, column: number): string {
  return `${columnName(column)}${row}`;
}

/** Sort comparator over order keys, tiebreaking on id so a duplicate
 *  (or degenerate) key stays deterministic. Board's convention. */
export function byOrder(
  a: { order: string; id: string },
  b: { order: string; id: string },
): number {
  return a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1;
}

/** An order key strictly between `a` and `b` (`""` = open bound),
 *  over the alphabet a–z. Copied from the board's lib — the repo's
 *  established fractional-ordering scheme. */
export function between(a: string, b: string): string {
  a = a || "";
  b = b || "";
  let out = "";
  for (let i = 0; ; i++) {
    const ca = i < a.length ? a.charCodeAt(i) : 96;
    const cb = b && i < b.length ? b.charCodeAt(i) : 123;
    if (ca === cb) {
      out += a[i];
      continue;
    }
    const m = Math.floor((ca + cb) / 2);
    if (m > ca) return out + String.fromCharCode(m);
    out += String.fromCharCode(ca === 96 ? 97 : ca);
    b = "";
  }
}

/** The order key for appending after the last of `list`. */
export function afterAll(list: { order: string; id: string }[]): string {
  const last = list.length ? [...list].sort(byOrder)[list.length - 1].order : "";
  return between(last, "");
}

/** Read the sheet claims for `subject` out of `root`'s data rows,
 *  sorted by order key. Rows for OTHER workbooks (a space can hold
 *  many tables; the directory view renders every instance) are
 *  filtered out here. */
export function readSheetRows(root: ParentNode, subject: string): SheetRow[] {
  const rows: SheetRow[] = [];
  for (const el of Array.from(root.querySelectorAll(".table-sheet-row"))) {
    const r = el as HTMLElement;
    const id = r.getAttribute("subject") ?? "";
    const table = r.dataset.table ?? "";
    if (!id || table !== subject) continue;
    rows.push({
      id,
      table,
      name: r.dataset.name ?? "",
      order: r.dataset.order ?? "",
    });
  }
  return rows.sort(byOrder);
}

/** Read the cell claims for the given sheets out of `root`'s data
 *  rows. Cells with malformed addresses are dropped (with a warning)
 *  rather than crashing the grid. */
export function readCellRows(root: ParentNode, sheetIds: Set<string>): CellRow[] {
  const rows: CellRow[] = [];
  for (const el of Array.from(root.querySelectorAll(".table-cell-row"))) {
    const r = el as HTMLElement;
    const id = r.getAttribute("subject") ?? "";
    const sheet = r.dataset.sheet ?? "";
    const at = r.dataset.at ?? "";
    if (!id || !sheetIds.has(sheet)) continue;
    if (parseAddress(at) === null) {
      console.warn(`[tonk-table] ignoring cell claim with bad address: ${at}`);
      continue;
    }
    rows.push({
      id,
      sheet,
      at,
      content: r.dataset.content ?? "",
      style: r.dataset.style ?? "",
    });
  }
  return rows;
}

/** Read the column-sizing claims for the given sheets. Malformed
 *  entries (bad letters, non-positive width) are dropped with a
 *  warning — hand-written claims must not crash the grid. */
export function readColumnRows(
  root: ParentNode,
  sheetIds: Set<string>,
): ColumnRow[] {
  const rows: ColumnRow[] = [];
  for (const el of Array.from(root.querySelectorAll(".table-column-row"))) {
    const r = el as HTMLElement;
    const id = r.getAttribute("subject") ?? "";
    const sheet = r.dataset.sheet ?? "";
    const at = r.dataset.at ?? "";
    const width = r.dataset.width ?? "";
    if (!id || !sheetIds.has(sheet)) continue;
    if (columnNumber(at) === null || !(Number(width) > 0)) {
      console.warn(`[tonk-table] ignoring column claim ${at}=${width}`);
      continue;
    }
    rows.push({ id, sheet, at, width });
  }
  return rows;
}

/** Read the row-sizing claims for the given sheets, same validation
 *  posture as columns. */
export function readRowSizeRows(
  root: ParentNode,
  sheetIds: Set<string>,
): RowSizeRow[] {
  const rows: RowSizeRow[] = [];
  for (const el of Array.from(root.querySelectorAll(".table-rowsize-row"))) {
    const r = el as HTMLElement;
    const id = r.getAttribute("subject") ?? "";
    const sheet = r.dataset.sheet ?? "";
    const at = r.dataset.at ?? "";
    const height = r.dataset.height ?? "";
    if (!id || !sheetIds.has(sheet)) continue;
    if (!/^[1-9][0-9]*$/.test(at) || !(Number(height) > 0)) {
      console.warn(`[tonk-table] ignoring row claim ${at}=${height}`);
      continue;
    }
    rows.push({ id, sheet, at, height });
  }
  return rows;
}

/** A cell-level change the grid must apply to (or has applied from)
 *  the engine. */
export interface CellDelta {
  sheet: string;
  at: string;
  /** `""` means cleared. */
  content: string;
}

/** Diff two `sheetId → (at → content)` snapshots into the deltas that
 *  turn `prev` into `next`. Used both inbound (rows changed → apply to
 *  engine) and outbound (undo/redo changed the engine → emit events). */
export function diffSnapshots(
  prev: Map<string, Map<string, string>>,
  next: Map<string, Map<string, string>>,
): CellDelta[] {
  const deltas: CellDelta[] = [];
  for (const [sheet, cells] of next) {
    const before = prev.get(sheet);
    for (const [at, content] of cells) {
      if ((before?.get(at) ?? "") !== content) deltas.push({ sheet, at, content });
    }
  }
  for (const [sheet, cells] of prev) {
    const after = next.get(sheet);
    for (const at of cells.keys()) {
      if (!after?.has(at) && (cells.get(at) ?? "") !== "") {
        deltas.push({ sheet, at, content: "" });
      }
    }
  }
  return deltas;
}

/** Optimistic-echo ledger (the board's `pending*` pattern): every
 *  outbound mutation is recorded here and consulted while reading the
 *  rows, so the grid answers instantly and reconciles when the claim
 *  round-trips. An entry clears on echo (row content matches) or
 *  after expiry — a claim the rules never accepted stops masking the
 *  store after `EXPIRY_MS`. */
export class PendingLedger {
  static readonly EXPIRY_MS = 8000;
  readonly #entries = new Map<string, { content: string; expires: number }>();
  readonly #now: () => number;

  constructor(now: () => number = () => Date.now()) {
    this.#now = now;
  }

  static key(sheet: string, at: string): string {
    return `${sheet} ${at}`;
  }

  record(sheet: string, at: string, content: string): void {
    this.#entries.set(PendingLedger.key(sheet, at), {
      content,
      expires: this.#now() + PendingLedger.EXPIRY_MS,
    });
  }

  /** Resolve what the grid should treat as current for a cell, given
   *  the store's row value. Clears the entry when the echo arrived
   *  (store caught up) or the entry expired. */
  resolve(sheet: string, at: string, rowContent: string): string {
    const key = PendingLedger.key(sheet, at);
    const entry = this.#entries.get(key);
    if (!entry) return rowContent;
    if (entry.content === rowContent || this.#now() > entry.expires) {
      this.#entries.delete(key);
      return rowContent;
    }
    return entry.content;
  }

  get size(): number {
    return this.#entries.size;
  }
}
