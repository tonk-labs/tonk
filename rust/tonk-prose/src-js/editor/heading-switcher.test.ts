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

test("what you typed is the first row", () => {
  const rows = suggestions(library, "Groceries");
  assert.equal(rows[0].create, true);
  assert.equal(rows[0].title, "Groceries");
});

test("the typed row leads even when matches follow", () => {
  const rows = suggestions(library, "Not");
  assert.equal(rows[0].title, "Not", "the typed text is first");
  assert.equal(rows[0].create, true);
  assert.ok(
    rows.slice(1).some((r) => r.title === "Notes" && !r.create),
    "and the matches follow it",
  );
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
  assert.equal(suggestions(library, "  Groceries  ")[0].title, "Groceries");
});

test("matched spans are returned for highlighting", () => {
  const [first] = rank(library, "wp");
  assert.deepEqual(first.spans, [0, 7]);
});
