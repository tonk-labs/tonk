// Minimal text-range diff for out-of-band value updates.
//
// When `setMarkdown` applies a value that didn't come from the local
// caret (a store round-trip, another device), replacing the whole
// document resets the selection. Instead we compute the smallest
// single span that changed and replace only that, so ProseMirror maps
// the caret through a tight range — it survives untouched text on
// either side, even inside the same block.
//
// The shape is the two-ended reduction every text editor uses for
// `setValue`: strip the common prefix, strip the common suffix, and
// whatever's left in the middle is the replaced span. It's O(n) and
// yields exactly one replaced range — which is all the caller needs
// to splice the document and let selection mapping do the rest. A
// full Levenshtein edit script (dominion's `EditDistance`) would give
// finer multi-run edits but costs O(n·m) and buys nothing here: one
// contiguous changed range preserves the caret just as well.

/** The single changed span between `a` and `b`, as offsets into each.
 *  `a[0, aFrom)` equals `b[0, bFrom)` (common prefix) and
 *  `a[aTo, len)` equals `b[bTo, len)` (common suffix); the middles
 *  `a[aFrom, aTo)` and `b[bFrom, bTo)` are what differ. When the two
 *  strings are equal every field collapses to the same point and the
 *  replaced span is empty. */
export interface TextDiff {
  aFrom: number;
  aTo: number;
  bFrom: number;
  bTo: number;
}

/** Compute the minimal changed span between `a` and `b`. Prefix and
 *  suffix scans are clamped so they never cross, so the two middles
 *  are always well-formed (`from <= to`). Unicode note: offsets are
 *  UTF-16 code units, matching JS string indexing and the positions a
 *  caller feeds to `String.prototype.slice`; a change that splits a
 *  surrogate pair still produces a valid (if one-unit-wider) span. */
export function diffText(a: string, b: string): TextDiff {
  const aLen = a.length;
  const bLen = b.length;

  // Common prefix.
  let prefix = 0;
  const maxPrefix = Math.min(aLen, bLen);
  while (prefix < maxPrefix && a.charCodeAt(prefix) === b.charCodeAt(prefix)) {
    prefix++;
  }

  // Common suffix, not overlapping the prefix on either string.
  let suffix = 0;
  const maxSuffix = Math.min(aLen - prefix, bLen - prefix);
  while (
    suffix < maxSuffix &&
    a.charCodeAt(aLen - 1 - suffix) === b.charCodeAt(bLen - 1 - suffix)
  ) {
    suffix++;
  }

  return {
    aFrom: prefix,
    aTo: aLen - suffix,
    bFrom: prefix,
    bTo: bLen - suffix,
  };
}
