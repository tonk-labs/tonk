import test from "node:test";
import assert from "node:assert/strict";
import { TableSession, type Reply } from "./document";

test("review: a slow duplicate open cannot overwrite heads after a committed edit", async () => {
  const reads: ((reply: Reply) => void)[] = [];
  const session = new TableSession({
    read: () => new Promise<Reply>((resolve) => reads.push(resolve)),
    write: async () => ({ heads: ["edited"], table: { sheets: [] } }),
  }, () => {});
  const first = session.open();
  const second = session.poll();
  reads[0]({ heads: ["base"], table: { sheets: [] } });
  await first;
  session.push([{ edit: "put", path: "sheets/s/cells/A1", value: "saved" }]);
  await session.flush();
  reads[1]({ heads: ["base"], table: { sheets: [] } });
  await second;
  assert.deepEqual(session.heads, ["edited"]);
});

test("table retries keep their original identity and do not absorb a later gesture", async () => {
  const writes: unknown[] = [];
  const session = new TableSession({
    read: async () => ({ heads: ["base"], table: { sheets: [] } }),
    write: async (heads, edits, request) => {
      writes.push(structuredClone({ heads, edits, request }));
      if (writes.length === 1) throw new Error("lost response");
      return { heads: [`h${writes.length}`], table: { sheets: [] } };
    },
  }, () => {});
  await session.open();
  session.push([{edit: "put", path: "sheets/s/cells/A1", value: "one"}]);
  await assert.rejects(session.flush());
  session.push([{edit: "put", path: "sheets/s/cells/A1", value: "two"}]);
  await session.flush();
  assert.deepEqual(writes[1], writes[0]);
  assert.notDeepEqual(writes[2], writes[0]);
  assert.equal(session.pending, 0);
});

test("table finish drains an in-flight gesture and its queue without touching a destroyed grid", async () => {
  let destroyed = false;
  let release!: (reply: Reply) => void;
  const writes: unknown[] = [];
  const session = new TableSession({
    read: async () => ({ heads: ["base"], table: { sheets: [] } }),
    write: async (heads, edits) => {
      writes.push(structuredClone({ heads, edits }));
      if (writes.length === 1) return new Promise<Reply>(resolve => { release = resolve; });
      return { heads: ["final"], table: { sheets: [] } };
    },
  }, () => { assert.equal(destroyed, false); });
  await session.open();
  session.push([{ edit: "put", path: "sheets/s/cells/A1", value: "one" }]);
  const writing = session.flush();
  session.push([{ edit: "put", path: "sheets/s/cells/B1", value: "two" }]);
  const finishing = session.finish();
  destroyed = true;
  release({ heads: ["merged"], table: { sheets: [] } });
  await Promise.all([writing, finishing]);
  assert.equal(writes.length, 2);
  assert.equal(session.pending, 0);
});

test("an old table poll cannot rewind a completed write", async () => {
  let release!: (reply: Reply) => void;
  let reads = 0;
  const session = new TableSession({
    read: async () => ++reads === 1
      ? { heads: ["base"], table: { sheets: [] } }
      : new Promise<Reply>(resolve => { release = resolve; }),
    write: async () => ({ heads: ["new"], table: { sheets: [] } }),
  }, () => {});
  await session.open();
  const polling = session.poll();
  session.push([{ edit: "put", path: "sheets/s/cells/A1", value: "new" }]);
  await session.flush();
  release({ heads: ["base"], table: { sheets: [] } });
  await polling;
  assert.deepEqual(session.heads, ["new"]);
});
