// The heavy chunk: ProseMirror assembly for `<tonk-prose>`. Loaded
// by the shell (`../index.ts`) via dynamic import on the first
// element connect — nothing here may be imported *statically* from
// the shell (types excepted).

import { EditorState, Plugin, Selection, TextSelection } from "prosemirror-state";
import { Slice } from "prosemirror-model";
import type { Node } from "prosemirror-model";
import { EditorView } from "prosemirror-view";
import { history } from "prosemirror-history";
import { dropCursor } from "prosemirror-dropcursor";
import { gapCursor } from "prosemirror-gapcursor";
import { schema } from "./schema";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { diffText } from "./diff";
import { isPlainTextblock } from "./markup";
import { buildInputRules } from "./input-rules";
import { buildKeymap, baseKeymap } from "./keymap";
import { keymap } from "prosemirror-keymap";
import { reparse } from "./reparse";
import { reveal } from "./reveal";
import { placeholder, placeholderKey } from "./placeholder";
import { imagePreview } from "./image-preview";
import { taskList } from "./task-list";
import { codeBlocks } from "./code-block";
import { headingSwitcher } from "./heading-switcher";
import type { EditorOptions, ProseEditor } from "./api";

/** Parse pasted plain text as markdown — Typora's paste behavior —
 *  and serialize copied/cut content back to real markdown.
 *
 *  Copy needs an explicit serializer: the default one flattens the
 *  *editor* doc (materialized marker text and all), so a copied list
 *  or blockquote loses its `- `/`> ` structure and inline markers leak
 *  through. Routing copy through `serializeMarkdown` (demarkup → the
 *  markdown serializer) reproduces the same clean markdown the whole
 *  document round-trips through. Pastes into code contexts keep the
 *  default (verbatim) handling. */
function markdownClipboardPlugin(): Plugin {
  return new Plugin({
    props: {
      clipboardTextParser(text, $context) {
        // Falsy result falls through to the default (verbatim) path.
        if ($context.parent.type.spec.code) return null as unknown as Slice;
        const doc = parseMarkdown(text);
        // maxOpen produces the natural open depths so pasting a
        // single paragraph mid-sentence splices inline.
        return Slice.maxOpen(doc.content);
      },
      clipboardTextSerializer(slice) {
        // Wrap the slice's fragment in a doc so the block serializer
        // (list/quote/heading prefixes) runs over it, then strip the
        // trailing newline `closeBlock` adds after the last block.
        const doc = schema.node("doc", null, slice.content);
        return serializeMarkdown(doc).replace(/\n$/, "");
      },
    },
  });
}

/** Document position just after the last leading top-level block that
 *  `a` and `b` share (compared by `Node.eq`). Positions returned are
 *  in `a`'s coordinate space, at block boundaries, so a replace from
 *  here never splits a block. */
function commonPrefixEnd(a: Node, b: Node): number {
  const n = Math.min(a.childCount, b.childCount);
  let pos = 0;
  for (let i = 0; i < n; i++) {
    if (!a.child(i).eq(b.child(i))) break;
    pos += a.child(i).nodeSize;
  }
  return pos;
}

/** Size (in document units) of the shared trailing block run of `a`
 *  and `b`, not overlapping the already-matched prefix that ends at
 *  `prefixEnd` in `a`. */
function commonSuffixLen(a: Node, b: Node, prefixEnd: number): number {
  let ai = a.childCount - 1;
  let bi = b.childCount - 1;
  let size = 0;
  while (ai >= 0 && bi >= 0) {
    const child = a.child(ai);
    // Don't let the suffix reach back past the prefix boundary.
    if (a.content.size - size - child.nodeSize < prefixEnd) break;
    if (!child.eq(b.child(bi))) break;
    size += child.nodeSize;
    ai--;
    bi--;
  }
  return size;
}

/** The single top-level child of `doc` spanning exactly `[from, to)`,
 *  when that range is one whole block — otherwise null (the range is
 *  empty, spans several blocks, or doesn't align to a block boundary).
 *  Used to decide whether the changed span can take the intra-block
 *  character diff. */
function singleTextblockAt(
  doc: Node,
  from: number,
  to: number,
): { node: Node } | null {
  if (to <= from) return null;
  let pos = 0;
  for (let i = 0; i < doc.childCount; i++) {
    const node = doc.child(i);
    if (pos === from && pos + node.nodeSize === to) {
      return node.isTextblock ? { node } : null;
    }
    pos += node.nodeSize;
    if (pos > from) break;
  }
  return null;
}

/** A textblock whose content is entirely text (markers included —
 *  they're text too). For such a block, document offsets equal text
 *  offsets, so a character-level text diff maps straight to positions.
 *  Mirrors the reparse loop's eligibility rule. */
function isPureText(node: Node): boolean {
  return isPlainTextblock(node);
}

export function createEditor(
  parent: HTMLElement,
  options: EditorOptions,
): ProseEditor {
  injectStylesheet(parent);

  let readOnly = options.readOnly;

  const state = EditorState.create({
    doc: parseMarkdown(options.doc),
    plugins: [
      // Reparse first: its plugin view sees every transaction, and
      // its state must be reset by flush metas before anything else
      // reads it. Order among the rest is not load-bearing except
      // code-block arrow keys before the base keymap.
      reparse(),
      // Before the keymaps, and only when the host asked for it.
      // `handleKeyDown` runs in plugin order, so the switcher has to come
      // first to claim Enter and the arrows while its list is open —
      // appended last, the base keymap would split the heading before the
      // switcher ever saw the key.
      ...(options.switcher ? headingSwitcher(options.switcher) : []),
      reveal(),
      buildInputRules(schema),
      ...codeBlocks(),
      buildKeymap(),
      keymap(baseKeymap),
      history(),
      dropCursor(),
      gapCursor(),
      placeholder(options.placeholder),
      imagePreview(),
      taskList(),
      markdownClipboardPlugin(),
    ],
  });

  const view = new EditorView(parent, {
    state,
    editable: () => !readOnly,
    attributes: { class: "md-doc" },
    dispatchTransaction(tr) {
      const next = view.state.apply(tr);
      view.updateState(next);
      if (tr.docChanged) {
        options.onChange(serializeMarkdown(next.doc));
      }
    },
  });

  return {
    view,

    getMarkdown(): string {
      return serializeMarkdown(view.state.doc);
    },

    setMarkdown(markdown: string): void {
      // A programmatic write that already matches the buffer (the
      // common case when our own `change` round-trips back through a
      // store) is a no-op — replacing the doc with an identical one
      // would still reset the selection and fight the user's caret.
      if (markdown === serializeMarkdown(view.state.doc)) return;

      const next = parseMarkdown(markdown);
      const current = view.state.doc;

      // Narrow to the span of top-level blocks that actually differ,
      // keeping a common prefix and suffix of blocks intact. An
      // out-of-band change usually touches one block, so the rest —
      // and any caret inside them — stay untouched.
      const from = commonPrefixEnd(current, next);
      const suffix = commonSuffixLen(current, next, from);
      const oldTo = current.content.size - suffix;
      const newTo = next.content.size - suffix;

      const tr = view.state.tr;

      // Intra-block refinement: when the differing span is exactly one
      // pure-text block on each side, diff the text and replace only
      // the changed characters. In a pure-text block document offsets
      // are text offsets (the reparse-loop invariant), so a caret in
      // the block's unchanged head or tail survives — the whole point
      // of an incremental update. `+1` steps past the block's opening
      // token to reach its text content.
      const oldBlock = singleTextblockAt(current, from, oldTo);
      const newBlock = singleTextblockAt(next, from, newTo);
      if (
        oldBlock &&
        newBlock &&
        isPureText(oldBlock.node) &&
        isPureText(newBlock.node)
      ) {
        const d = diffText(oldBlock.node.textContent, newBlock.node.textContent);
        const base = from + 1;
        const insert = newBlock.node.textContent.slice(d.bFrom, d.bTo);
        if (insert.length > 0) {
          tr.replaceWith(base + d.aFrom, base + d.aTo, schema.text(insert));
        } else {
          tr.delete(base + d.aFrom, base + d.aTo);
        }
      } else {
        tr.replaceWith(from, oldTo, next.slice(from, newTo).content);
      }

      const mapped = tr.mapping.map(view.state.selection.head, -1);
      const target = Math.min(Math.max(mapped, 0), tr.doc.content.size);
      tr.setSelection(TextSelection.create(tr.doc, target));
      view.dispatch(tr);
    },

    setReadOnly(next: boolean): void {
      readOnly = next;
      // Re-run the `editable` prop.
      view.setProps({});
    },

    setPlaceholder(text: string): void {
      view.dispatch(view.state.tr.setMeta(placeholderKey, text));
    },

    focus(): void {
      view.focus();
    },

    caretToEnd(): void {
      // `Selection.atEnd` finds the last valid text position, which is
      // not the same as the document's size: the end of a doc whose last
      // node is a code block or a list sits inside that node, and a raw
      // `doc.content.size` would be an invalid position there.
      const { doc, tr } = view.state;
      view.dispatch(tr.setSelection(Selection.atEnd(doc)).scrollIntoView());
    },

    destroy(): void {
      view.destroy();
    },
  };
}

/** Editor document stylesheet, injected once per root node (the
 *  shell's shadow root in practice). Everything routes through the
 *  `--tonk-prose-*` variables the shell defines on the host. */
function injectStylesheet(parent: HTMLElement): void {
  const root = parent.getRootNode();
  const container = root instanceof ShadowRoot ? root : document.head;
  if (container.querySelector("style[data-tonk-prose-editor]")) return;
  const style = document.createElement("style");
  style.setAttribute("data-tonk-prose-editor", "");
  style.textContent = EDITOR_STYLESHEET;
  container.append(style);
}

const EDITOR_STYLESHEET = `
  .md-doc {
    font-family: var(--tonk-prose-font);
    font-size: var(--tonk-prose-font-size);
    line-height: 1.6;
    color: var(--tonk-prose-fg);
    padding: var(--tonk-prose-padding);
    max-width: var(--tonk-prose-max-width);
    margin: 0 auto;
    outline: none;
    white-space: pre-wrap;
    word-wrap: break-word;
    caret-color: var(--tonk-prose-accent);
    height: 100%;
    box-sizing: border-box;
  }

  .md-doc ::selection { background: var(--tonk-prose-selection); }

  .md-doc p { margin: 0 0 0.75em; }

  .md-doc h1, .md-doc h2, .md-doc h3,
  .md-doc h4, .md-doc h5, .md-doc h6 {
    font-family: var(--tonk-prose-heading-font);
    line-height: 1.25;
    margin: 1.1em 0 0.5em;
    font-weight: 650;
  }
  .md-doc h1:first-child, .md-doc h2:first-child,
  .md-doc h3:first-child, .md-doc p:first-child { margin-top: 0; }
  .md-doc h1 { font-size: 1.9em; }
  .md-doc h2 { font-size: 1.5em; }
  .md-doc h3 { font-size: 1.25em; }
  .md-doc h4 { font-size: 1.05em; }
  .md-doc h5 { font-size: 1em; }
  .md-doc h6 { font-size: 0.9em; color: var(--tonk-prose-fg-muted); }

  .md-doc blockquote {
    margin: 0 0 0.75em;
    padding: 0 1em;
    border-left: 3px solid var(--tonk-prose-border);
    color: var(--tonk-prose-blockquote);
  }

  .md-doc ul, .md-doc ol { padding-left: 1.6em; margin: 0 0 0.75em; }
  .md-doc li > p { margin-bottom: 0.25em; }

  /* Task-list checkbox: the rendered stand-in for a hidden "[ ] "
     source prefix (task-list.ts). Nudged into the gutter so the
     text still aligns with the bullet column, and drawn in the
     accent color. The list bullet on an item that owns a checkbox
     reads as redundant, but it stays (Typora keeps it too) while
     the checkbox sits inline just before the text. */
  .md-doc .md-task-checkbox {
    appearance: none;
    -webkit-appearance: none;
    width: 1em;
    height: 1em;
    margin: 0 0.35em 0 0;
    vertical-align: -0.12em;
    border: 1.5px solid var(--tonk-prose-border);
    border-radius: 3px;
    background: var(--tonk-prose-bg);
    cursor: pointer;
    position: relative;
    flex: none;
  }
  .md-doc .md-task-checkbox:checked {
    background: var(--tonk-prose-accent);
    border-color: var(--tonk-prose-accent);
  }
  .md-doc .md-task-checkbox:checked::after {
    content: "";
    position: absolute;
    left: 0.28em;
    top: 0.08em;
    width: 0.22em;
    height: 0.5em;
    border: solid #fff;
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
  }

  .md-doc hr {
    border: none;
    border-top: 2px solid var(--tonk-prose-border);
    margin: 1.5em 0;
  }

  .md-doc a {
    color: var(--tonk-prose-link);
    text-decoration: underline;
    text-underline-offset: 0.15em;
    cursor: pointer;
  }
  .md-doc a:hover { text-decoration-thickness: 2px; }

  .md-doc del {
    text-decoration: line-through;
    text-decoration-color: var(--tonk-prose-fg-muted);
  }

  .md-doc mark {
    background: var(--tonk-prose-highlight-bg);
    color: var(--tonk-prose-highlight-fg);
    border-radius: 2px;
    padding: 0 0.1em;
  }

  .md-doc code {
    font-family: var(--tonk-prose-mono);
    font-size: 0.875em;
    background: var(--tonk-prose-code-bg);
    color: var(--tonk-prose-code-fg);
    border-radius: 4px;
    padding: 0.15em 0.35em;
  }

  .md-doc img {
    max-width: 100%;
    border-radius: var(--tonk-prose-radius);
  }

  /* Expanded-image previews: the widget that follows the (usually
     hidden) source text. Block display gives the picture its own
     line, Typora-style; the source text above it reads as a
     caption while editing. */
  .md-image-preview {
    display: block;
    margin: 0.25em 0;
  }
  .md-image-broken {
    min-width: 6em;
    min-height: 2.5em;
    border: 1px dashed var(--tonk-prose-border);
    color: var(--tonk-prose-fg-muted);
    font-size: 0.8em;
  }

  /* Embedded code editors. The <tonk-code> element brings its own
     frame; the wrapper only spaces it. The plain fallback mimics a
     quiet code surface. */
  .md-code-block { margin: 0 0 0.75em; }
  .md-code-block-plain {
    font-family: var(--tonk-prose-mono);
    font-size: 0.875em;
    background: var(--tonk-prose-code-bg);
    color: var(--tonk-prose-code-fg);
    border: 1px solid var(--tonk-prose-border);
    border-radius: var(--tonk-prose-radius);
    padding: 0.75em 1em;
    white-space: pre-wrap;
    overflow-x: auto;
  }
  .md-code-block-plain code {
    background: none;
    padding: 0;
    font-size: inherit;
  }

  /* ——— The Typora reveal ———
     Markers are literal text (.md-markup spans), hidden at rest and
     revealed by two selection-driven decorations (see reveal.ts):

       md-active — the caret's textblock. Reveals *block* markers
                   (.md-block: the heading "# " prefix), which
                   belong to the whole line.
       md-edit   — the caret's edit range (the mark run it touches).
                   Reveals *inline* markers only for that span: the
                   caret in bold shows its "**", in a link its "["
                   and "](url)"; other spans stay rendered.

     ProseMirror may paint the md-edit class onto the marker span
     itself or onto a decoration span nested inside it, so both
     shapes are matched. Reveal only while the editor is focused —
     an unfocused editor reads as rendered markdown, Typora-style. */
  .md-markup {
    display: none;
    color: var(--tonk-prose-marker);
    font-weight: 400;
    font-style: normal;
  }
  /* Reveal a marker as real, full-size text when the caret is in its
     block (block markers) or touches its span (inline markers, via the
     md-edit edit range). The edit range anchors from either side, so a
     span's markers appear as the caret approaches its boundary — and
     because the revealed marker is real visible text, native caret
     movement and typing stop at the boundary with nothing hidden to
     skip over. ProseMirror may paint md-edit onto the marker span
     itself or onto a decoration nested inside it, so both are matched. */
  .ProseMirror-focused .md-active .md-markup.md-block,
  .ProseMirror-focused .md-markup.md-edit,
  .ProseMirror-focused .md-markup:has(.md-edit) {
    display: inline;
  }

  /* ——— Block elements become their markdown while the caret is in
     them ———
     A list item's first paragraph carries its dash/number marker (and,
     for a todo, the checkbox source) as md-block marker text, revealed
     by the rule above when the item's block is md-active. To read as
     pure markdown source in that state, the native list marker (the
     bullet/number the browser draws) and the rendered checkbox widget
     must step aside — otherwise the line shows both the native bullet
     and the dash marker, plus a checkbox next to the source. Suppress
     them for the active item only; every other item stays richly
     rendered. */
  .md-doc li:has(> .md-active) {
    list-style: none;
  }
  .md-doc li:has(> .md-active) > .md-active .md-task-checkbox {
    display: none;
  }

  /* Placeholder (empty doc): ghost text via CSS content. */
  .tonk-prose-empty::before {
    content: attr(data-placeholder);
    color: var(--tonk-prose-fg-muted);
    font-style: italic;
    float: left;
    height: 0;
    pointer-events: none;
  }

  /* ProseMirror needs these for correct behavior. */
  .ProseMirror { position: relative; }
  .ProseMirror-hideselection *::selection { background: transparent; }
  /* No outline on a selected node. A code cell is selected whenever the
     caret enters it, and a ring around the box reads as an error state
     rather than a cursor. The block highlight below carries "where am I"
     instead, across the whole block rather than one node of it. */
  .ProseMirror-selectednode { outline: none; }
  .ProseMirror-gapcursor {
    display: none;
    pointer-events: none;
    position: absolute;
  }
  .ProseMirror-gapcursor:after {
    content: "";
    display: block;
    position: absolute;
    top: -2px;
    width: 20px;
    border-top: 1px solid var(--tonk-prose-fg);
    animation: ProseMirror-cursor-blink 1.1s steps(2, start) infinite;
  }
  @keyframes ProseMirror-cursor-blink { to { visibility: hidden; } }
  .ProseMirror-focused .ProseMirror-gapcursor { display: block; }
`;
