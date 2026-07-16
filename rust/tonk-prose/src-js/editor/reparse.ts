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
import type { EditorView } from "prosemirror-view";
import { parseCleanMarkdown } from "./markdown";
import { materializeBlock, isPlainTextblock } from "./markup";

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

/** Reparse one dirty block inside `tr`; returns true when the block
 *  was swapped. `pos` must point at the block in `tr.doc`. */
function reparseBlock(tr: Transaction, pos: number): boolean {
  const node = tr.doc.nodeAt(pos);
  if (!node || !isPlainTextblock(node)) return false;

  const source = node.textContent;
  if (!source) return false;

  const parsed = parseCleanMarkdown(source);
  if (parsed.childCount !== 1) return false;
  const block = materializeBlock(parsed.firstChild!);

  if (!block.isTextblock) return false;
  if (block.eq(node)) return false;
  // Lossless-swap guard: the text the user sees must be preserved
  // exactly (this rules out escape-resolution differences and any
  // parse that ate characters).
  if (block.textContent !== source) return false;

  const $pos = tr.doc.resolve(pos);
  const index = $pos.index();
  if (!$pos.parent.canReplaceWith(index, index + 1, block.type)) return false;

  // Caret offset within the block, if the selection head is inside.
  // Pure text ⇒ document offsets are text offsets ⇒ the offset is
  // valid in the swapped block too.
  const { head } = tr.selection;
  const inBlock = head >= pos + 1 && head <= pos + node.nodeSize - 1;
  const offset = inBlock ? head - (pos + 1) : null;

  tr.replaceWith(pos, pos + node.nodeSize, block);

  if (offset !== null) {
    const target = Math.min(pos + 1 + offset, pos + block.nodeSize - 1);
    tr.setSelection(TextSelection.create(tr.doc, target));
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
