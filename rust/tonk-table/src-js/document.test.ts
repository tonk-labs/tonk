import { test } from "node:test";
import assert from "node:assert/strict";

import {
  TableSession,
  conflictsOf,
  editsOf,
  parseHeads,
  rowsOf,
  type Reply,
  type TableEdit,
  type TableSnapshot,
} from "./document";

const sheet = (cells: Record<string, string>, conflicts: string[] = []) => ({
  id: "s1",
  name: "Sheet1",
  order: "m",
  cells,
  styles: {},
  widths: { B: 120 },
  heights: {},
  conflicts,
});

test("turns a workbook into the rows the grid reconciles", () => {
  const rows = rowsOf("id:table/book", { sheets: [sheet({ A1: "Item", D2: "=B2*C2" })] });
  assert.deepEqual(rows.sheets, [{ id: "s1", table: "id:table/book", name: "Sheet1", order: "m" }]);
  assert.deepEqual(
    rows.cells.map((cell) => [cell.id, cell.sheet, cell.at, cell.content]),
    [
      ["s1!A1", "s1", "A1", "Item"],
      ["s1!D2", "s1", "D2", "=B2*C2"],
    ],
  );
  assert.deepEqual(rows.columns, [{ id: "s1!col:B", sheet: "s1", at: "B", width: "120" }]);
});

test("maps every grid mutation event to a path edit", () => {
  const mint = () => "new";
  assert.deepEqual(editsOf("createcell", { cellSheet: "s1", cellAt: "B2", cellContent: "5" }, mint), [
    { edit: "put", path: "sheets/s1/cells/B2", value: "5" },
  ]);
  // An edit names the ROW id the snapshot produced; it maps back with no lookup.
  assert.deepEqual(editsOf("editcell", { editCell: "s1!B2", editContent: "6" }, mint), [
    { edit: "put", path: "sheets/s1/cells/B2", value: "6" },
  ]);
  assert.deepEqual(editsOf("clearcell", { clearCell: "s1!B2" }, mint), [
    { edit: "remove", path: "sheets/s1/cells/B2" },
  ]);
  assert.deepEqual(editsOf("createsheet", { sheetName: "Two", sheetOrder: "t" }, mint), [
    { edit: "put", path: "sheets/new/name", value: "Two" },
    { edit: "put", path: "sheets/new/order", value: "t" },
  ]);
  assert.deepEqual(editsOf("renamesheet", { renameSheet: "s1", renameName: "Renamed" }, mint), [
    { edit: "put", path: "sheets/s1/name", value: "Renamed" },
  ]);
  assert.deepEqual(editsOf("resizecolumn", { resizeColumn: "s1!col:B", resizeWidth: "140" }, mint), [
    { edit: "put", path: "sheets/s1/widths/B", value: 140 },
  ]);
  assert.deepEqual(editsOf("createrow", { rowSheet: "s1", rowAt: "2", rowHeight: 30 }, mint), [
    { edit: "put", path: "sheets/s1/heights/2", value: 30 },
  ]);
  assert.deepEqual(editsOf("selectionchange", {}, mint), [], "an observability event is not an edit");
  assert.deepEqual(editsOf("editcell", { editCell: "no-bang" }, mint), [], "a malformed id edits nothing");
});

test("lists conflicted cells", () => {
  assert.deepEqual(conflictsOf({ sheets: [sheet({ B2: "x" }, ["B2"])] }), ["s1!B2"]);
});

/** A host whose document is a key/value map, merged like the real one. */
class FakeHost {
  cells: Record<string, string> = {};
  version = 0;
  writes: TableEdit[][] = [];
  fail = false;

  #reply(): Reply {
    const table: TableSnapshot = { sheets: [sheet({ ...this.cells })] };
    return { heads: [`h${this.version}`], table };
  }

  remote(at: string, content: string): void {
    this.cells[at] = content;
    this.version++;
  }

  read = async (): Promise<Reply> => this.#reply();

  write = async (_heads: string[], edits: TableEdit[]): Promise<Reply> => {
    if (this.fail) throw new Error("offline");
    this.writes.push(edits);
    for (const edit of edits) {
      const at = edit.path.split("/").pop() as string;
      if (edit.edit === "put") this.cells[at] = String(edit.value);
      else delete this.cells[at];
    }
    this.version++;
    return this.#reply();
  };
}

test("sends one gesture as one write", async () => {
  const host = new FakeHost();
  const applied: TableSnapshot[] = [];
  const session = new TableSession(host, (table) => applied.push(table));
  await session.open();

  // A paste commits many cells in one tick.
  session.push(editsOf("createcell", { cellSheet: "s1", cellAt: "A1", cellContent: "1" }, () => "x"));
  session.push(editsOf("createcell", { cellSheet: "s1", cellAt: "A2", cellContent: "2" }, () => "x"));
  await session.flush();

  assert.equal(host.writes.length, 1, "one write, so one automerge change");
  assert.equal(host.writes[0].length, 2);
  assert.deepEqual(applied.at(-1)?.sheets[0].cells, { A1: "1", A2: "2" });
});

test("brings in a remote cell on poll and keeps edits across a failed write", async () => {
  const host = new FakeHost();
  let latest: TableSnapshot = { sheets: [] };
  const session = new TableSession(host, (table) => (latest = table));
  await session.open();

  host.remote("C3", "remote");
  await session.poll();
  assert.equal(latest.sheets[0].cells.C3, "remote");

  host.fail = true;
  session.push(editsOf("createcell", { cellSheet: "s1", cellAt: "A1", cellContent: "kept" }, () => "x"));
  await assert.rejects(session.flush());
  assert.equal(session.pending, 1, "a failed write keeps its edits");

  host.fail = false;
  await session.flush();
  assert.equal(latest.sheets[0].cells.A1, "kept");
  assert.equal(latest.sheets[0].cells.C3, "remote");
});

test("a session pinned to a past version takes no edits and does not follow the branch", async () => {
  const host = new FakeHost();
  host.remote("A1", "past");
  let latest: TableSnapshot = { sheets: [] };
  const session = new TableSession(host, (table) => (latest = table), { pinned: true });
  await session.open();
  assert.equal(latest.sheets[0].cells.A1, "past");

  session.push(editsOf("createcell", { cellSheet: "s1", cellAt: "B1", cellContent: "x" }, () => "x"));
  await session.flush();
  assert.equal(host.writes.length, 0);

  host.remote("A1", "later");
  await session.poll();
  assert.equal(latest.sheets[0].cells.A1, "past");
  assert.deepEqual(parseHeads("ab cd"), ["ab", "cd"]);
});

test("waits for a workbook whose bytes are not here yet", async () => {
  const host = new FakeHost();
  host.remote("A1", "late");
  let ready = false;
  let opens = 0;
  let latest: TableSnapshot = { sheets: [] };
  const session = new TableSession(
    {
      read: async () => {
        if (!ready) throw new Error("the document is missing changes the heads name");
        return host.read();
      },
      write: host.write,
    },
    (table) => (latest = table),
    { onOpen: () => opens++ },
  );
  await assert.rejects(session.open());
  assert.equal(session.opened, false);
  await assert.rejects(session.poll(), "a poll retries the open");

  ready = true;
  await session.poll();
  assert.equal(session.opened, true);
  assert.equal(latest.sheets[0].cells.A1, "late");
  assert.equal(opens, 1);
});

test("a workbook in a newer format is shown and never written", async () => {
  const host = new FakeHost();
  host.remote("A1", "newer");
  let latest: TableSnapshot = { sheets: [] };
  const session = new TableSession(
    { read: async () => ({ ...(await host.read()), readonly: true }), write: host.write },
    (table) => (latest = table),
  );
  await session.open();
  assert.equal(latest.sheets[0].cells.A1, "newer");
  assert.equal(session.readonly, true);
  session.push(editsOf("createcell", { cellSheet: "s1", cellAt: "B1", cellContent: "x" }, () => "x"));
  await session.flush();
  assert.equal(host.writes.length, 0);
});
