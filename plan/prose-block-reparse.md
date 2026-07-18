# Prose block-structure reparse

## Problem

`<tonk-prose>` materializes block prefixes as literal marker text so a
textblock's `textContent` is its full markdown line: `# `, `> `, `- `,
`1. `, `[ ] ` are all hidden marker spans, revealed when the caret is in
the block ("block becomes its markdown while editing").

The reparse loop (reparse.ts) reparses **one textblock at a time** and
swaps it in place, with a special path that lifts/sinks the *blockquote*
nesting around a single block when its leading `>`s change. This cannot
express the edits the marker model implies:

- Delete `> ` on one line of a multi-paragraph blockquote → that line
  should leave the quote, splitting it into quote / plain / quote. The
  single-block lift bails when the blockquote has >1 child.
- Delete `- ` / `- [ ] ` on a list item → the item should leave the list
  (become a plain paragraph, splitting the list). No list lift exists;
  the current code drops the marker but keeps the item in the list, so
  the item ends up markerless and mangled.
- Type `> ` / `- ` at the start of a plain paragraph between two list/
  quote blocks → should join into the neighbor structure.

The prefixes are authoritative (like `#`), so the block's structure must
be **derived from its text**, not patched per-edit.

## Approach: reparse the whole wrapper

When an edit dirties a textblock that lives inside a block wrapper
(blockquote or list — possibly nested), reparse the **outermost such
wrapper** as one unit:

1. Find the outermost enclosing blockquote/list/list_item chain — the
   "structural root" containing the dirty block.
2. Take that root's full source: each descendant textblock's
   `textContent` (which already carries its `>`/`-`/`1.`/`[ ]` prefixes
   and indentation) joined by `\n`. Because every prefix is literal
   text, this join is valid markdown.
3. `parseCleanMarkdown(source)` → a clean subtree, then
   `materializeDoc` → the re-materialized subtree (markers re-stamped
   consistently from structure).
4. Lossless guard: the re-materialized subtree's joined `textContent`
   must equal the source (no characters eaten). Bail otherwise.
5. Replace the root range `[before(rootDepth), after(rootDepth)]` with
   the fragment of parsed top-level blocks. A split naturally produces
   several siblings (quote / paragraph / quote); an in-place inline edit
   produces one identical-structured root and the swap is a no-op-ish
   replace.
6. Restore the caret by text-offset within the root (offsets are stable
   because the join preserves text exactly — the same trick the
   per-block path already uses).

This subsumes the blockquote lift/sink special case (delete/insert `>`
just changes the parsed structure) and adds list membership for free.

### Plain textblocks (not in a wrapper)

A textblock with no blockquote/list ancestor keeps the existing
single-block path (heading toggle, inline marks). Only wrapper-enclosed
edits use the whole-wrapper path. This keeps the common case (typing in a
paragraph) cheap.

### Indentation / nested lists

Source lines for nested items carry leading indentation so markdown-it
re-derives nesting. `materializeDoc` already threads list depth; the
join must reproduce the indentation markdown-it expects (2 spaces per
level for bullets). Start with one level; guard (step 4) catches any
lossy nesting and bails to leave structure untouched rather than corrupt
it.

## Tests (markup.test.ts + reparse coverage)

- delete `>` on middle quote line → quote / paragraph / quote
- delete `- ` on middle bullet → list / paragraph / list
- delete `- [ ] ` on a todo → plain paragraph out of the list
- type `> ` on a paragraph between two quotes → single merged quote
- inline edit inside a quoted/list line → structure unchanged, mark applied
- round-trip idempotence for all the above
