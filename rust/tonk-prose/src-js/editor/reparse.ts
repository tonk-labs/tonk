// The Typora engine (allusion's `prosemirror-marked/Plugin.js` +
// `EditRange.js`, modernized): because markdown markers are literal
// text (markup.ts), a textblock's `textContent` IS its markdown
// source. So instead of one input rule per syntax, we watch which
// textblocks the user edits, and — after a short debounce — reparse
// each dirty block's text and swap the block in when its structure
// changed:
//
//   type `**bold**`   → paragraph text reparses to a strong span
//                       (markers re-materialized as markup text)
//   delete one `*`    → span degrades back to literal text
//   type `# ` prefix  → paragraph becomes a heading
//   delete the `#`    → heading becomes a paragraph
//
// Guards keep the swap conservative and lossless:
//   • only pure-text blocks participate (images / hard breaks have
//     no faithful text form — see isPlainTextblock);
//   • the source must parse to exactly one node the parent accepts;
//   • the swap must preserve `textContent` exactly. This is also
//     what makes caret restoration trivial: in a pure-text block,
//     document offsets ARE text offsets, so the caret's offset
//     survives the swap verbatim (allusion's Position.js trick,
//     for free).
//
// History: transactions produced by undo/redo do NOT dirty blocks —
// otherwise Ctrl-Z on a conversion would immediately re-convert,
// making the conversion impossible to escape. (`undoInputRule`
// stays useful for the block-level input rules only.)

import { Plugin, PluginKey, TextSelection } from "prosemirror-state";
import type { EditorState, Transaction } from "prosemirror-state";
import type { Node, ResolvedPos } from "prosemirror-model";
import type { EditorView } from "prosemirror-view";
import { schema } from "./schema";
import { reparseBlockLines } from "./markdown";
import { isPlainTextblock } from "./markup";

/** Collapse `_`/`__` emphasis delimiters to their `*`/`**` equivalents —
 *  the form materialization emits — so the reparse guard compares source
 *  and rebuilt text on equal footing (a typed `_for_` materializes to
 *  `*for*`). Only underscores that could be delimiters matter; mapping all
 *  of them is safe for the guard because both sides are mapped the same. */
function normalizeEmphasis(text: string): string {
  return text.replace(/_/g, "*");
}

/** A block wrapper — a blockquote or a list — whose structure is
 *  derived from the `>`/`-`/`1.` prefixes its textblocks carry. */
function isWrapper(node: Node): boolean {
  return (
    node.type === schema.nodes.blockquote ||
    node.type === schema.nodes.bullet_list ||
    node.type === schema.nodes.ordered_list
  );
}

/** How long after the last edit the reparse fires. Allusion used
 *  80ms; slightly longer keeps the swap out of fast typing bursts. */
const DEBOUNCE_MS = 120;

type ReparseState = {
  /** Start positions of textblocks touched since the last flush,
   *  mapped forward through every transaction. */
  dirty: number[];
};

export const reparseKey = new PluginKey<ReparseState>("tonk-prose-reparse");

/** Meta tag on transactions the flush itself dispatches, so they
 *  don't re-dirty the blocks they just rewrote. */
const FLUSH = "flush";

/** Collect the start positions of eligible textblocks overlapping
 *  the ranges `tr` changed, expressed in `tr.doc` coordinates. */
function touchedBlocks(tr: Transaction): number[] {
  const found = new Set<number>();
  const doc = tr.doc;
  tr.mapping.maps.forEach((stepMap, i) => {
    const rest = tr.mapping.slice(i + 1);
    stepMap.forEach((_oldStart, _oldEnd, newStart, newEnd) => {
      const from = Math.max(0, Math.min(rest.map(newStart, -1), doc.content.size));
      const to = Math.max(0, Math.min(rest.map(newEnd, 1), doc.content.size));
      doc.nodesBetween(from, to, (node, pos) => {
        if (node.isTextblock) {
          if (isPlainTextblock(node)) found.add(pos);
          return false;
        }
        return true;
      });
    });
  });
  return [...found];
}

/** The outermost wrapper (blockquote/list) enclosing the block at
 *  `$block` (resolved INSIDE the block), or null when the block has no
 *  wrapper ancestor. Returns the wrapper's depth and its doc range. */
function outermostWrapper(
  $block: ResolvedPos,
): { depth: number; from: number; to: number } | null {
  let wrapperDepth = -1;
  for (let d = $block.depth - 1; d >= 1; d--) {
    if (isWrapper($block.node(d))) wrapperDepth = d;
  }
  if (wrapperDepth < 0) return null;
  return {
    depth: wrapperDepth,
    from: $block.before(wrapperDepth),
    to: $block.after(wrapperDepth),
  };
}

/** Every textblock's `textContent`, in document order, within `node` —
 *  the per-line markdown source (each line carries its `>`/`-`/`1.`
 *  prefix from materialization). Empty lines are kept: an empty list
 *  item / quote line is a real line of source. */
function blockSourceLines(node: Node): string[] {
  const lines: string[] = [];
  node.descendants((child) => {
    if (child.isTextblock) {
      lines.push(child.textContent);
      return false;
    }
    return true;
  });
  return lines;
}

/** Reparse the WHOLE wrapper enclosing the dirty block: rebuild its
 *  structure from the `>`/`-`/`1.` prefixes its lines carry, so
 *  dropping a prefix lifts that line out (splitting the wrapper) and
 *  adding one joins it in. Returns true when the wrapper was replaced. */
function reparseWrapper(
  tr: Transaction,
  wrapper: { from: number; to: number },
): boolean {
  const node = tr.doc.nodeAt(wrapper.from);
  if (!node) return false;
  const rawLines = blockSourceLines(node);
  if (rawLines.some((line) => !line)) return false;

  // A line that no longer starts with a block prefix but still carries the
  // LEADING space left by deleting a marker char (e.g. `- x` → ` x` after
  // the `-` is deleted) is being lifted out. Strip that one leading space
  // before reparsing so the line reads as a clean plain paragraph — but
  // leave every other character (crucially TRAILING spaces the user is
  // actively typing) untouched, so the reparse never eats them.
  const lines = rawLines.map((line) =>
    /^ (?![ >])/.test(line) && !/^(?:> |[-*+] |\d+[.)] |#{1,6} )/.test(line)
      ? line.slice(1)
      : line,
  );

  const fragment = reparseBlockLines(lines);
  // Lossless guard: the rebuilt blocks must carry EXACTLY the same text as
  // the (leading-space-normalized) lines we fed in. Trailing whitespace is
  // preserved — a trailing space is the user typing before the next word,
  // never a lift artifact — so it must survive the round-trip, or the swap
  // would delete it mid-typing. A mismatch means the parse changed content
  // and the swap would corrupt the wrapper, so bail.
  const rebuilt = schema.node("doc", null, fragment);
  if (blockSourceLines(rebuilt).join("\n") !== lines.join("\n")) return false;
  if (
    fragment.childCount === node.childCount &&
    node.eq(fragment.child(0)) &&
    fragment.childCount === 1
  ) {
    return false; // unchanged
  }

  const { head } = tr.selection;
  const inWrapper = head > wrapper.from && head < wrapper.to;
  // Caret as a text offset from the wrapper's first inner position — the
  // join preserves text exactly, so the offset survives the rebuild.
  const textOffset = inWrapper ? textOffsetAt(tr.doc, wrapper.from, head) : null;

  const $from = tr.doc.resolve(wrapper.from);
  const parentIndex = $from.index();
  if (
    !$from.parent.canReplace(
      parentIndex,
      parentIndex + 1,
      fragment,
    )
  ) {
    return false;
  }
  tr.replaceWith(wrapper.from, wrapper.to, fragment);

  if (textOffset !== null) {
    // The replaced region spans the whole fragment, which may now be
    // several sibling blocks (a split). Search that entire span, not
    // just the first block, or the caret clamps into the wrong sibling.
    const target = positionAtTextOffset(
      tr.doc,
      wrapper.from,
      wrapper.from + fragment.size,
      textOffset,
    );
    if (target !== null) tr.setSelection(TextSelection.create(tr.doc, target));
  }
  return true;
}

/** Text length between doc position `from` (a node boundary) and `to`,
 *  counting only text characters — position arithmetic that survives a
 *  structural rebuild preserving the same text. */
function textOffsetAt(doc: Node, from: number, to: number): number {
  return doc.textBetween(from, to, "", "").length;
}

/** The doc position `offset` text characters into the block region
 *  `[from, to)`, or null if it runs past the region's end. Walks
 *  textblocks accumulating their text length; the caret lands inside the
 *  block where the running count reaches `offset`. `to` must bound the
 *  whole region (all sibling blocks a split produced), not just the
 *  first — otherwise the offset clamps into the wrong sibling. */
function positionAtTextOffset(
  doc: Node,
  from: number,
  to: number,
  offset: number,
): number | null {
  let remaining = offset;
  let result: number | null = null;
  let lastBlockEnd: number | null = null;
  doc.nodesBetween(from, to, (node, pos) => {
    if (result !== null) return false;
    if (node.isTextblock) {
      const len = node.content.size;
      // `remaining < len` lands strictly inside this block. `=== len`
      // is a block boundary: prefer the START of the next textblock (a
      // split moved the caret's line to a new block), so consume the
      // block and fall through; only if no later block exists does the
      // trailing end (recorded below) become the answer.
      if (remaining < len) {
        result = pos + 1 + remaining;
        return false;
      }
      remaining -= len;
      lastBlockEnd = pos + 1 + len;
      return false;
    }
    return true;
  });
  // Offset landed exactly at (or past) the region's last block boundary.
  return result ?? lastBlockEnd;
}

/** Reparse one dirty block inside `tr`. A block inside a wrapper
 *  (blockquote/list) reparses the whole wrapper (structure follows the
 *  prefixes); a plain textblock reparses in place (heading toggle,
 *  inline marks). `pos` must point at the block in `tr.doc`. */
function reparseBlock(tr: Transaction, pos: number): boolean {
  const node = tr.doc.nodeAt(pos);
  if (!node || !isPlainTextblock(node)) return false;

  const $block = tr.doc.resolve(pos + 1);
  const wrapper = outermostWrapper($block);
  if (wrapper) return reparseWrapper(tr, wrapper);

  // Plain textblock (no wrapper): reparse its single line. `textContent`
  // is the block's full markdown source, so a heading toggle, an inline
  // mark, OR a newly-typed `> `/`- ` prefix (which parses to a wrapper)
  // all reparse from the same text. Splice in whatever top-level blocks
  // result — usually one textblock, but a gained prefix yields a wrapper.
  const source = node.textContent;
  if (!source) return false;
  const fragment = reparseBlockLines([source]);
  if (fragment.childCount === 0) return false;

  // Lossless guard: the swap must preserve the visible text. Compare the
  // rebuilt block's `textContent` to the source with emphasis delimiters
  // normalized the way materialization does — an `_`-delimited run becomes
  // `*` (materializeInline only knows the `*` form), so a strict `=== source`
  // would reject a typed `_for_` and underscore emphasis could never
  // convert. Normalizing `_`→`*` on both sides lets it convert (rendered as
  // italic, serialized back as `*for*`) while a parse that ate or reordered
  // characters still mismatches and is rejected.
  const rebuilt = schema.node("doc", null, fragment);
  const rebuiltText = rebuilt.textBetween(0, rebuilt.content.size, "", "");
  if (normalizeEmphasis(rebuiltText) !== normalizeEmphasis(source)) {
    return false;
  }
  if (fragment.childCount === 1 && fragment.child(0).eq(node)) return false;

  const $pos = tr.doc.resolve(pos);
  const index = $pos.index();
  if (!$pos.parent.canReplace(index, index + 1, fragment)) return false;

  const { head } = tr.selection;
  const inBlock = head >= pos + 1 && head <= pos + node.nodeSize - 1;
  const offset = inBlock ? head - (pos + 1) : null;

  tr.replaceWith(pos, pos + node.nodeSize, fragment);
  if (offset !== null) {
    const target = positionAtTextOffset(
      tr.doc,
      pos,
      pos + fragment.size,
      offset,
    );
    if (target !== null) tr.setSelection(TextSelection.create(tr.doc, target));
  }
  return true;
}

/** Was this transaction produced by the history plugin (undo/redo)?
 *  prosemirror-history tags its transactions with the `"history$"`
 *  meta key. */
function isHistoryTr(tr: Transaction): boolean {
  return tr.getMeta("history$") !== undefined;
}

export function reparse(): Plugin<ReparseState> {
  const plugin: Plugin<ReparseState> = new Plugin<ReparseState>({
    key: reparseKey,
    state: {
      init: () => ({ dirty: [] }),
      apply(tr, prev): ReparseState {
        if (tr.getMeta(reparseKey) === FLUSH) return { dirty: [] };
        let dirty = prev.dirty;
        if (tr.docChanged) {
          // Map surviving positions forward, then add the blocks
          // this transaction touched (unless it's an undo/redo —
          // re-converting undone text would trap the user).
          dirty = dirty
            .map((pos) => tr.mapping.mapResult(pos))
            .filter((r) => !r.deleted)
            .map((r) => r.pos);
          if (!isHistoryTr(tr)) {
            for (const pos of touchedBlocks(tr)) {
              if (!dirty.includes(pos)) dirty.push(pos);
            }
          }
        }
        return dirty === prev.dirty ? prev : { dirty };
      },
    },
    view(view: EditorView) {
      let timer: ReturnType<typeof setTimeout> | null = null;

      const flush = () => {
        timer = null;
        const state: EditorState = view.state;
        const { dirty } = reparseKey.getState(state)!;
        if (!dirty.length) return;
        // Descending order: swapping a later block never shifts an
        // earlier block's position.
        const order = [...new Set(dirty)].sort((a, b) => b - a);
        const tr = state.tr;
        let changed = false;
        for (const pos of order) {
          changed = reparseBlock(tr, pos) || changed;
        }
        tr.setMeta(reparseKey, FLUSH);
        if (changed || dirty.length) view.dispatch(tr);
      };

      return {
        update() {
          const { dirty } = reparseKey.getState(view.state)!;
          if (!dirty.length) return;
          if (timer !== null) clearTimeout(timer);
          timer = setTimeout(flush, DEBOUNCE_MS);
        },
        destroy() {
          if (timer !== null) clearTimeout(timer);
        },
      };
    },
  });
  return plugin;
}

/** Run the reparse flush synchronously against `state` and return the
 *  resulting state — the same work the debounced plugin view does, made
 *  callable for tests (headless, there is no EditorView to drive the
 *  timer). Reparses every dirty block once, in descending position
 *  order. */
export function flushReparse(state: EditorState): EditorState {
  const { dirty } = reparseKey.getState(state)!;
  if (!dirty.length) return state;
  const order = [...new Set(dirty)].sort((a, b) => b - a);
  const tr = state.tr;
  for (const pos of order) reparseBlock(tr, pos);
  tr.setMeta(reparseKey, FLUSH);
  return state.apply(tr);
}
