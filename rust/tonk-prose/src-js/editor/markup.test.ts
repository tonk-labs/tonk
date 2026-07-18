import { test } from "node:test";
import assert from "node:assert/strict";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { schema } from "./schema";
import type { Node } from "prosemirror-model";

/** The literal text a materialized block renders as (its `textContent`),
 *  block by block — this is the markdown source line the reparse loop
 *  and the reveal rely on. */
function lines(doc: Node): string[] {
  const out: string[] = [];
  doc.descendants((node) => {
    if (node.isTextblock) {
      out.push(node.textContent);
      return false;
    }
    return true;
  });
  return out;
}

test("a bullet item's first line carries a `- ` block marker", () => {
  const doc = parseMarkdown("- one\n- two");
  assert.deepEqual(lines(doc), ["- one", "- two"]);
});

test("an ordered item's first line carries its `N. ` marker", () => {
  const doc = parseMarkdown("1. one\n2. two\n3. three");
  assert.deepEqual(lines(doc), ["1. one", "2. two", "3. three"]);
});

test("an ordered list starting at 3 numbers from its start", () => {
  const doc = parseMarkdown("3. three\n4. four");
  assert.deepEqual(lines(doc), ["3. three", "4. four"]);
});

test("a task item carries `- [ ] ` (list marker + checkbox source)", () => {
  const doc = parseMarkdown("- [ ] todo\n- [x] done");
  assert.deepEqual(lines(doc), ["- [ ] todo", "- [x] done"]);
});

test("a blockquote carries `> ` on every paragraph", () => {
  // A blank `>` line separates the quote into two paragraphs; each
  // gets its own `> ` marker.
  const doc = parseMarkdown("> a quote\n>\n> hello");
  assert.deepEqual(lines(doc), ["> a quote", "> hello"]);
});

test("a heading still carries its `# ` marker", () => {
  const doc = parseMarkdown("## Title");
  assert.deepEqual(lines(doc), ["## Title"]);
});

test("bullet list round-trips to markdown (markers stripped on serialize)", () => {
  const src = "- one\n\n- two";
  assert.equal(serializeMarkdown(parseMarkdown(src)).trim(), "- one\n\n- two");
});

test("ordered list round-trips to markdown", () => {
  const src = "1. one\n\n2. two";
  const out = serializeMarkdown(parseMarkdown(src)).trim();
  assert.equal(out, "1. one\n\n2. two");
});

test("task list round-trips to markdown", () => {
  const src = "- [ ] todo\n\n- [x] done";
  const out = serializeMarkdown(parseMarkdown(src)).trim();
  assert.equal(out, "- [ ] todo\n\n- [x] done");
});

test("blockquote round-trips to markdown", () => {
  const src = "> a quote\n>\n> hello";
  const out = serializeMarkdown(parseMarkdown(src)).trim();
  assert.equal(out, "> a quote\n>\n> hello");
});

test("highlight (==) round-trips and carries a `==` marker", () => {
  const doc = parseMarkdown("a ==hi== b");
  const hasHighlight: boolean[] = [];
  doc.descendants((n) => {
    if (n.isText && n.marks.some((m) => m.type.name === "highlight")) {
      hasHighlight.push(true);
    }
  });
  assert.ok(hasHighlight.length > 0, "== should parse to a highlight mark");
  assert.equal(serializeMarkdown(doc).trim(), "a ==hi== b");
});

test("bold inside a bullet item round-trips", () => {
  const src = "- a **bold** item";
  assert.deepEqual(lines(parseMarkdown(src)), ["- a **bold** item"]);
  assert.equal(serializeMarkdown(parseMarkdown(src)).trim(), src);
});

test("a task item with inline code round-trips", () => {
  const src = "- [x] run `code`";
  assert.deepEqual(lines(parseMarkdown(src)), ["- [x] run `code`"]);
  assert.equal(serializeMarkdown(parseMarkdown(src)).trim(), src);
});

test("a quoted bullet list carries both `> ` and `- `", () => {
  const doc = parseMarkdown("> - one\n> - two");
  assert.deepEqual(lines(doc), ["> - one", "> - two"]);
});

test("materialize→serialize→parse is idempotent for a mixed doc", () => {
  const src = "# Title\n\n- one\n\n- [ ] todo\n\n> quote";
  const once = serializeMarkdown(parseMarkdown(src)).trim();
  const twice = serializeMarkdown(parseMarkdown(once)).trim();
  assert.equal(once, twice);
});

void schema;
