import test from "node:test";
import assert from "node:assert/strict";
import { DocumentSession, textEditor, type Reply } from "./document";

test("review: a slow duplicate open cannot replace text typed after the first open", async () => {
  let value = "";
  const reads: ((reply: Reply<string>) => void)[] = [];
  const session = new DocumentSession(
    {
      read: () => new Promise<Reply<string>>((resolve) => reads.push(resolve)),
      write: async () => { throw new Error("not used"); },
    },
    textEditor(() => value, (next) => { value = next; }),
  );
  const first = session.open();
  const second = session.poll(); // poll timer ticks before the first open returns
  reads[0]({ heads: ["base"], content: "original" });
  await first;
  value = "original and my unsaved edit";
  reads[1]({ heads: ["base"], content: "original" });
  await second;
  assert.equal(value, "original and my unsaved edit");
  assert.equal(session.dirty, true);
});

test("retry retains the exact gesture while newer typing waits for its own request", async () => {
  let value = "";
  const writes: { heads: string[]; edits: unknown[]; request: unknown }[] = [];
  const session = new DocumentSession({
    read: async () => ({ heads: ["base"], content: "" }),
    write: async (heads, edits, request) => {
      writes.push(structuredClone({ heads, edits, request }));
      if (writes.length === 1) throw new Error("reply lost after commit");
      const content = (edits[0] as { text: string }).text;
      return { heads: [`h${writes.length}`], local: [`h${writes.length}`], content };
    },
  }, textEditor(() => value, next => { value = next; }));
  await session.open();
  value = "one";
  await assert.rejects(session.flush());
  value = "one two";
  await session.flush();
  assert.deepEqual(writes[1], writes[0]);
  assert.notDeepEqual(writes[2].request, writes[0].request);
  assert.deepEqual(writes[2].heads, ["h2"]);
  assert.equal(value, "one two");
  assert.equal(session.dirty, false);
});

test("finish drains text typed during an in-flight write without reading the destroyed editor", async () => {
  let value = "";
  let destroyed = false;
  let release!: (reply: Reply<string>) => void;
  const sent: string[] = [];
  const session = new DocumentSession({
    read: async () => ({ heads: ["base"], content: "" }),
    write: async (_heads, edits) => {
      const text = (edits[0] as {text: string}).text;
      sent.push(text);
      if (sent.length === 1) return new Promise<Reply<string>>(resolve => { release = resolve; });
      return { heads: ["final"], content: text };
    },
  }, textEditor(() => { assert.equal(destroyed, false); return value; }, next => { assert.equal(destroyed, false); value = next; }));
  await session.open();
  value = "one";
  const writing = session.flush();
  value = "one two";
  const finishing = session.finish();
  destroyed = true;
  release({ heads: ["merged"], local: ["one"], content: "one remote" });
  await Promise.all([writing, finishing]);
  assert.deepEqual(sent, ["one", "one two"]);
});

test("finish does not overwrite remote text included in the pending reply", async () => {
  let value = "";
  let release!: (reply: Reply<string>) => void;
  let writes = 0;
  const session = new DocumentSession({
    read: async () => ({ heads: ["base"], content: "" }),
    write: () => { writes++; return new Promise<Reply<string>>(resolve => { release = resolve; }); },
  }, textEditor(() => value, next => { value = next; }));
  await session.open(); value = "local";
  const writing = session.flush(); const finishing = session.finish();
  release({ heads: ["merged"], local: ["local"], content: "local remote" });
  await Promise.all([writing, finishing]);
  assert.equal(writes, 1);
});

test("an old prose poll cannot rewind a completed write", async () => {
  let value = "";
  let release!: (reply: Reply<string>) => void;
  let reads = 0;
  const session = new DocumentSession({
    read: async () => ++reads === 1
      ? { heads: ["base"], content: "old" }
      : new Promise<Reply<string>>(resolve => { release = resolve; }),
    write: async () => ({ heads: ["new"], content: "new" }),
  }, textEditor(() => value, next => { value = next; }));
  await session.open();
  const polling = session.poll();
  value = "new";
  await session.flush();
  release({ heads: ["base"], content: "old" });
  await polling;
  assert.equal(value, "new");
  assert.deepEqual(session.heads, ["new"]);
});
