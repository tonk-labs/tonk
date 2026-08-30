import { test } from "node:test";
import assert from "node:assert/strict";
import { exact, rank, suggestions, type Candidate } from "./heading-switcher";

const library: Candidate[] = [
  { title: "Weekly Planning", href: "notebook/id:a" },
  { title: "Notes", href: "notebook/id:b" },
  { title: "Swap Pointers", href: "notebook/id:c" },
];

test("an initialism ranks the intended notebook first", () => {
  const [first] = rank(library, "wp");
  assert.equal(first.title, "Weekly Planning");
});

test("a query matching nothing ranks nothing", () => {
  assert.deepEqual(rank(library, "zzzz"), []);
});

test("a blank query offers the whole library", () => {
  assert.equal(rank(library, "").length, library.length);
});

test("an exact title is found regardless of case and space", () => {
  assert.equal(exact(library, "  notes ")?.href, "notebook/id:b");
});

test("a prefix is not an exact match", () => {
  assert.equal(exact(library, "Not"), null);
});

test("the create row is offered last for an unmatched name", () => {
  const rows = suggestions(library, "Groceries");
  const last = rows[rows.length - 1];
  assert.equal(last.create, true);
  assert.equal(last.title, "Groceries");
});

test("the create row is offered even when something matches", () => {
  // `Not` fuzzy-matches Notes, but the author may still mean a new
  // notebook called `Not` — so both are on offer.
  const rows = suggestions(library, "Not");
  assert.ok(rows.some((r) => r.title === "Notes" && !r.create));
  assert.ok(rows.some((r) => r.title === "Not" && r.create));
});

test("no create row when the title already exists exactly", () => {
  const rows = suggestions(library, "Notes");
  assert.ok(!rows.some((r) => r.create), "an exact title is an open, not a create");
});

test("no create row for a blank query", () => {
  assert.ok(!suggestions(library, "   ").some((r) => r.create));
});

test("the create row carries the trimmed title", () => {
  const rows = suggestions(library, "  Groceries  ");
  assert.equal(rows[rows.length - 1].title, "Groceries");
});

test("matched spans are returned for highlighting", () => {
  const [first] = rank(library, "wp");
  assert.deepEqual(first.spans, [0, 7]);
});
