// Subsequence matching for the heading switcher, in the manner of
// Obsidian's quick switcher: typing `wkpl` finds "Weekly Planning".
//
// A plain substring test makes you remember how a title STARTS, or at
// least an unbroken run of it. A subsequence test only asks that you
// remember the letters in order, which is how people actually recall a
// name they have not seen in a week.

/** Where a query's characters landed in a title, for highlighting. */
export type Match = {
  /** Indices into the title that the query matched, ascending. */
  spans: number[];
  /** Higher is better. Ranks matches against each other, nothing else. */
  score: number;
};

/**
 * Match `query` against `title` as a subsequence, case-insensitively.
 *
 * Returns `null` when a character of the query is missing. A blank query
 * matches everything with an empty span list, so an untouched heading
 * offers the whole library rather than nothing.
 *
 * Scoring favours, in order: matches at the start of the title, matches at
 * word boundaries, and runs of adjacent characters. That is what makes
 * `wp` rank "Weekly Planning" above "Swap Pointers".
 */
export function fuzzy(title: string, query: string): Match | null {
  const needle = query.trim().toLowerCase();
  if (needle === "") return { spans: [], score: 0 };

  const hay = title.toLowerCase();
  const spans: number[] = [];
  let score = 0;
  let at = 0;

  for (const ch of needle) {
    const found = hay.indexOf(ch, at);
    if (found === -1) return null;
    // A character right after the previous one is a run: worth more than a
    // scattered hit, so contiguous typing beats coincidence.
    if (spans.length > 0 && found === spans[spans.length - 1] + 1) score += 8;
    // The first character of a word — what people actually type when they
    // abbreviate.
    if (found === 0) score += 16;
    else if (/[\s\-_/]/.test(hay[found - 1] ?? "")) score += 12;
    // Earlier is better, mildly, so a short title with the match up front
    // beats a long one that happens to contain it late.
    score += Math.max(0, 8 - found);
    spans.push(found);
    at = found + 1;
  }

  // Shorter titles win ties: "Notes" over "Notes on the notes".
  score -= Math.max(0, title.length - needle.length) / 32;
  return { spans, score };
}
