import { test } from "node:test";
import assert from "node:assert/strict";
import { fuzzy } from "./fuzzy";

test("a blank query matches everything", () => {
  const m = fuzzy("Weekly Planning", "");
  assert.deepEqual(m, { spans: [], score: 0 });
});

test("an initialism matches across words", () => {
  assert.ok(fuzzy("Weekly Planning", "wp"));
  assert.ok(fuzzy("Weekly Planning", "wkpl"));
});

test("a missing character does not match", () => {
  assert.equal(fuzzy("Weekly Planning", "wpz"), null);
});

test("order matters: the query is a subsequence, not a bag", () => {
  assert.equal(fuzzy("Weekly Planning", "pw"), null);
});

test("matching is case-insensitive", () => {
  assert.ok(fuzzy("Weekly Planning", "WEEKLY"));
  assert.ok(fuzzy("weekly planning", "Planning"));
});

test("spans point at the matched characters", () => {
  const m = fuzzy("Notes", "nts");
  assert.deepEqual(m?.spans, [0, 2, 4]);
});

test("word-boundary hits outrank mid-word ones", () => {
  const boundary = fuzzy("Weekly Planning", "wp");
  const midword = fuzzy("Swap Pointers", "wp");
  assert.ok(boundary && midword);
  assert.ok(
    boundary.score > midword.score,
    `expected initials to win: ${boundary.score} vs ${midword.score}`,
  );
});

test("a contiguous run outranks a scattered match", () => {
  const run = fuzzy("Planning", "plan");
  const scattered = fuzzy("Personal Ledger And Notes", "plan");
  assert.ok(run && scattered);
  assert.ok(
    run.score > scattered.score,
    `expected the run to win: ${run.score} vs ${scattered.score}`,
  );
});

test("a shorter title breaks a tie", () => {
  const short = fuzzy("Notes", "notes");
  const long = fuzzy("Notes on the notes", "notes");
  assert.ok(short && long);
  assert.ok(short.score > long.score);
});
