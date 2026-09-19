import { test } from "node:test";
import assert from "node:assert/strict";

import {
  DocumentSession,
  parseHeads,
  textEditor,
  sameHeads,
  type Reply,
  type TextEdit,
} from "./document";

/** A host that behaves like the real one for text: a write is merged
 *  on top of the heads it names. Text is modelled as a list of tokens
 *  so a merge is a union, which is enough to catch a lost or doubled
 *  edit. */
class FakeHost {
  #seq = 0;
  /** heads id -> the tokens visible at those heads */
  #versions = new Map<string, string[]>([["h0", []]]);
  branch = "h0";
  /** Resolvers of writes the test is holding back. */
  held: (() => void)[] = [];
  hold = false;

  #tokens(heads: string[]): string[] {
    const out = new Set<string>();
    for (const head of heads) for (const token of this.#versions.get(head) ?? []) out.add(token);
    return [...out].sort();
  }

  #mint(tokens: string[]): string {
    const id = `h${++this.#seq}`;
    this.#versions.set(id, tokens);
    return id;
  }

  /** Someone else edits the branch. */
  remote(token: string): void {
    this.branch = this.#mint([...this.#tokens([this.branch]), token].sort());
  }

  read = async (): Promise<Reply<string>> => ({
    heads: [this.branch],
    content: this.#tokens([this.branch]).join(" "),
  });

  write = async (heads: string[], edits: TextEdit[]): Promise<Reply<string>> => {
    if (this.hold) await new Promise<void>((resolve) => this.held.push(resolve));
    const text = edits[0].text;
    const local = this.#mint(text === "" ? [] : text.split(" "));
    void heads;
    this.branch = this.#mint(this.#tokens([this.branch, local]));
    return { heads: [this.branch], local: [local], content: this.#tokens([this.branch]).join(" ") };
  };
}

function setup() {
  const host = new FakeHost();
  let text = "";
  const applied: string[] = [];
  const session = new DocumentSession<string, TextEdit>(
    host,
    textEditor(
      () => text,
      (next) => {
        text = next;
        applied.push(next);
      },
    ),
  );
  return {
    host,
    session,
    applied,
    type: (next: string) => {
      text = next;
    },
    text: () => text,
  };
}

test("opens at the branch's version", async () => {
  const s = setup();
  s.host.remote("a");
  await s.session.open();
  assert.equal(s.text(), "a");
  assert.equal(s.session.dirty, false);
});

test("sends an edit and stays put when nothing else changed", async () => {
  const s = setup();
  await s.session.open();
  s.type("a");
  assert.equal(s.session.dirty, true);
  await s.session.flush();
  assert.equal(s.text(), "a");
  assert.equal(s.session.dirty, false);
  assert.deepEqual(s.applied, [""], "its own edit is not applied back onto the editor");
});

test("merges a remote edit that landed before the write", async () => {
  const s = setup();
  await s.session.open();
  s.host.remote("remote");
  s.type("local");
  await s.session.flush();
  assert.equal(s.text(), "local remote", "the reply carries the merged content");
});

test("keeps what was typed during a round trip and loses nothing", async () => {
  const s = setup();
  await s.session.open();
  s.host.hold = true;
  s.type("one");
  const flushing = s.session.flush();
  await Promise.resolve();

  // Typed while the write is in flight, and a remote edit lands too.
  s.type("one two");
  s.host.remote("remote");
  s.host.hold = false;
  s.host.held.shift()?.();
  await flushing;

  assert.equal(s.text(), "one remote two", "both local edits and the remote edit are present, once");
  assert.equal(s.session.dirty, false);
});

test("a poll brings in a remote change, but never over unsent edits", async () => {
  const s = setup();
  await s.session.open();
  s.host.remote("remote");
  s.type("unsent");
  await s.session.poll();
  assert.equal(s.text(), "unsent", "a poll does not replace unsent edits");

  await s.session.flush();
  assert.equal(s.text(), "remote unsent");

  s.host.remote("later");
  await s.session.poll();
  assert.equal(s.text(), "later remote unsent");
});

test("an unchanged poll does not touch the editor", async () => {
  const s = setup();
  await s.session.open();
  const before = s.applied.length;
  await s.session.poll();
  assert.equal(s.applied.length, before);
});

test("a session pinned to a past version never writes and never polls", async () => {
  const host = new FakeHost();
  host.remote("past");
  let text = "";
  let writes = 0;
  const session = new DocumentSession<string, TextEdit>(
    {
      read: host.read,
      write: (heads, edits) => {
        writes++;
        return host.write(heads, edits);
      },
    },
    textEditor(
      () => text,
      (next) => (text = next),
    ),
    { pinned: true },
  );
  await session.open();
  assert.equal(text, "past");

  text = "past typed";
  await session.flush();
  assert.equal(writes, 0, "a past version takes no edits");

  host.remote("later");
  await session.poll();
  assert.equal(text, "past typed", "a past version does not follow the branch");
});

test("waits for a document whose bytes are not here yet", async () => {
  const host = new FakeHost();
  host.remote("late");
  let text = "";
  let ready = false;
  let opens = 0;
  const session = new DocumentSession<string, TextEdit>(
    {
      read: async () => {
        if (!ready) throw new Error("the document is missing changes the heads name");
        return host.read();
      },
      write: host.write,
    },
    textEditor(
      () => text,
      (next) => (text = next),
    ),
    { onOpen: () => opens++ },
  );
  await assert.rejects(session.open());
  assert.equal(session.opened, false);

  text = "typed into a closed document";
  await session.flush();
  assert.equal(host.branch, "h1", "an unopened document sends nothing");

  await assert.rejects(session.poll(), "a poll retries the open");
  ready = true;
  await session.poll();
  assert.equal(session.opened, true);
  assert.equal(text, "late");
  assert.equal(opens, 1);
});

test("a document in a newer format is shown and never written", async () => {
  const host = new FakeHost();
  host.remote("newer");
  let text = "";
  let writes = 0;
  const session = new DocumentSession<string, TextEdit>(
    {
      read: async () => ({ ...(await host.read()), readonly: true }),
      write: (heads, edits) => {
        writes++;
        return host.write(heads, edits);
      },
    },
    textEditor(
      () => text,
      (next) => (text = next),
    ),
  );
  await session.open();
  assert.equal(text, "newer");
  assert.equal(session.readonly, true);
  text = "newer typed";
  await session.flush();
  assert.equal(writes, 0);
});

test("reads the heads of an `at` attribute", () => {
  assert.deepEqual(parseHeads(" ab  cd\n"), ["ab", "cd"]);
  assert.deepEqual(parseHeads(null), []);
});

test("compares heads as sets", () => {
  assert.equal(sameHeads(["a", "b"], ["b", "a"]), true);
  assert.equal(sameHeads(["a"], ["a", "b"]), false);
});
