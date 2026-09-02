// Code blocks as embedded code editors.
//
// When the `<tonk-code>` custom element is defined on the page, each
// code block mounts one as a node view — inheriting its CodeMirror
// editing, its *lazily-loaded* per-language chunks (the `language`
// attribute resolves to `tonk-code-lang-<id>.js` on demand), and its
// theming. When `<tonk-code>` is not defined, the node view falls
// back to a plain `<pre>` that ProseMirror edits natively, so the
// block is always editable — the embedded editor is a progressive
// enhancement, decided per node view at draw time.
//
// Sync follows the official ProseMirror CodeMirror example: an
// `updating` flag breaks echo loops, and CodeMirror→ProseMirror
// edits are applied as a minimal prefix/suffix diff so undo history
// and collaborative mapping stay fine-grained.

import type { Node } from "prosemirror-model";
import { Plugin, Selection, TextSelection } from "prosemirror-state";
import type { Command, EditorState } from "prosemirror-state";
import type { EditorView, NodeView } from "prosemirror-view";
import { keymap } from "prosemirror-keymap";
import { schema } from "./schema";

type GetPos = () => number | undefined;

/** Minimal shape of the `<tonk-code>` element we rely on (the real
 *  type lives in the tonk-code package; duplicating three members
 *  beats a cross-package dependency). */
type TonkCodeLike = HTMLElement & {
  value: string;
  /** CodeMirror EditorView — power-user getter `<tonk-code>` exposes. */
  readonly view: {
    state: {
      doc: { length: number; lines: number; lineAt(pos: number): { number: number } };
      selection: { main: { empty: boolean; head: number; from: number; to: number } };
    };
    dispatch(spec: { selection: { anchor: number; head?: number } }): void;
    focus(): void;
  } | null;
};

/** Fence info words that name no language pack of their own but should
 *  still resolve to one. `dialog` is what an author writes for a tonk
 *  query cell; the pack that exists is `dialog-yaml`. Without the alias
 *  the embedded editor raises "no language pack for dialog" and the block
 *  renders unhighlighted. */
const LANGUAGE_ALIASES: Record<string, string> = {
  dialog: "dialog-yaml",
};

/** First word of the fence info string → tonk-code `language` id. */
function languageOf(node: Node): string | null {
  const params = (node.attrs.params as string) ?? "";
  const id = params.trim().split(/\s+/)[0];
  if (!id) return null;
  return LANGUAGE_ALIASES[id] ?? id;
}

/** Prefix/suffix diff between two strings; null when equal. */
function computeChange(
  prev: string,
  next: string,
): { from: number; to: number; text: string } | null {
  if (prev === next) return null;
  let start = 0;
  let prevEnd = prev.length;
  let nextEnd = next.length;
  while (
    start < prevEnd &&
    prev.charCodeAt(start) === next.charCodeAt(start)
  ) {
    start++;
  }
  while (
    prevEnd > start &&
    nextEnd > start &&
    prev.charCodeAt(prevEnd - 1) === next.charCodeAt(nextEnd - 1)
  ) {
    prevEnd--;
    nextEnd--;
  }
  return { from: start, to: prevEnd, text: next.slice(start, nextEnd) };
}

/** Node view embedding `<tonk-code>`. */
class TonkCodeBlockView implements NodeView {
  readonly dom: HTMLElement;
  #node: Node;
  readonly #outer: EditorView;
  readonly #getPos: GetPos;
  readonly #editor: TonkCodeLike;
  /** Guards the CM⇄PM sync against echo loops. */
  #updating = false;
  /** Whether a Backspace on an empty code block has already been pressed
   *  once, so the next one removes the block. See `#maybeEscape`. */
  #armedForDelete = false;

  constructor(node: Node, outer: EditorView, getPos: GetPos) {
    this.#node = node;
    this.#outer = outer;
    this.#getPos = getPos;

    this.#editor = document.createElement("tonk-code") as TonkCodeLike;
    this.#editor.setAttribute("value", node.textContent);
    const language = languageOf(node);
    if (language) this.#editor.setAttribute("language", language);

    const dom = document.createElement("div");
    dom.className = "md-code-block";
    dom.append(this.#editor);
    this.dom = dom;

    this.#editor.addEventListener("change", (event) => {
      if (this.#updating) return;
      const value = (event as CustomEvent<{ value: string }>).detail.value;
      this.#forwardChange(value);
    });
    // Composed keydowns from inside the shadow root retarget to the
    // host element — enough to implement boundary escapes.
    this.#editor.addEventListener("keydown", (event) =>
      this.#maybeEscape(event),
    );
  }

  /** Apply a CodeMirror edit to the ProseMirror doc as a minimal
   *  replace step. */
  #forwardChange(value: string): void {
    const pos = this.#getPos();
    if (pos === undefined) return;
    const change = computeChange(this.#node.textContent, value);
    if (!change) return;
    const start = pos + 1;
    const tr = this.#outer.state.tr.replaceWith(
      start + change.from,
      start + change.to,
      change.text ? schema.text(change.text) : [],
    );
    this.#outer.dispatch(tr);
  }

  /** Boundary escapes: arrows at the document edges move the outer
   *  selection past the block; Backspace in an empty editor turns
   *  the block back into a paragraph (Typora's behavior). */
  #maybeEscape(event: KeyboardEvent): void {
    const cm = this.#editor.view;
    if (!cm) return;
    const { doc, selection } = cm.state;
    const main = selection.main;
    if (!main.empty) return;

    if (event.key === "Backspace" && doc.length === 0) {
      event.preventDefault();
      // Deleting the last CHARACTER must not also delete the BLOCK. Emptying
      // a code block is ordinary editing — you are about to type something
      // else — so a single Backspace that happens to land on an empty editor
      // would drop the fence out from under the caret. Require a second,
      // deliberate press: the first arms, the next one converts.
      //
      // The flag clears on any other key, so typing after emptying the block
      // leaves it a code block, and it clears on blur so a stale arm cannot
      // surprise a later visit.
      if (this.#armedForDelete) {
        this.#armedForDelete = false;
        this.#replaceWithParagraph();
      } else {
        this.#armedForDelete = true;
      }
      return;
    }
    // Any other key in the block disarms: only consecutive Backspaces at an
    // empty document mean "remove this block".
    this.#armedForDelete = false;

    let dir: -1 | 1 | 0 = 0;
    if (
      (event.key === "ArrowUp" && doc.lineAt(main.head).number === 1) ||
      (event.key === "ArrowLeft" && main.head === 0)
    ) {
      dir = -1;
    } else if (
      (event.key === "ArrowDown" &&
        doc.lineAt(main.head).number === doc.lines) ||
      (event.key === "ArrowRight" && main.head === doc.length)
    ) {
      dir = 1;
    }
    if (dir === 0) return;

    const pos = this.#getPos();
    if (pos === undefined) return;
    const target = dir < 0 ? pos : pos + this.#node.nodeSize;
    const outerSel = Selection.near(
      this.#outer.state.doc.resolve(target),
      dir,
    );
    // Nowhere to go. When escaping DOWNWARD off a code block that is the
    // last block, there's no paragraph to land in — so make one, so the
    // block is never a trap. (Escaping upward past the very first block
    // has no natural target and just stays put.)
    if (outerSel.head === this.#outer.state.selection.head) {
      if (dir > 0 && target === this.#outer.state.doc.content.size) {
        event.preventDefault();
        const tr = this.#outer.state.tr;
        const paragraph = schema.nodes.paragraph.create();
        tr.insert(target, paragraph);
        tr.setSelection(TextSelection.create(tr.doc, target + 1));
        this.#outer.dispatch(tr.scrollIntoView());
        this.#outer.focus();
      }
      return;
    }
    event.preventDefault();
    this.#outer.dispatch(
      this.#outer.state.tr.setSelection(outerSel).scrollIntoView(),
    );
    this.#outer.focus();
  }

  #replaceWithParagraph(): void {
    const pos = this.#getPos();
    if (pos === undefined) return;
    const tr = this.#outer.state.tr.replaceWith(
      pos,
      pos + this.#node.nodeSize,
      schema.nodes.paragraph.create(),
    );
    tr.setSelection(TextSelection.create(tr.doc, pos + 1));
    this.#outer.dispatch(tr);
    this.#outer.focus();
  }

  update(node: Node): boolean {
    if (node.type !== this.#node.type) return false;
    const prevLanguage = languageOf(this.#node);
    this.#node = node;
    const language = languageOf(node);
    if (language !== prevLanguage) {
      if (language) this.#editor.setAttribute("language", language);
      else this.#editor.removeAttribute("language");
    }
    const text = node.textContent;
    if (this.#editor.value !== text) {
      this.#updating = true;
      try {
        this.#editor.value = text;
      } finally {
        this.#updating = false;
      }
    }
    return true;
  }

  setSelection(anchor: number, head: number): void {
    const cm = this.#editor.view;
    if (cm) {
      cm.dispatch({ selection: { anchor, head } });
      cm.focus();
    } else {
      this.#editor.focus();
    }
  }

  selectNode(): void {
    this.#editor.focus();
  }

  stopEvent(): boolean {
    // Everything that happens inside the embedded editor is its
    // business; ProseMirror must not double-handle it.
    return true;
  }

  ignoreMutation(): boolean {
    return true;
  }
}

/** Fallback node view: a plain `<pre>` ProseMirror edits natively.
 *  Used when `<tonk-code>` isn't defined on the page. */
class PlainCodeBlockView implements NodeView {
  readonly dom: HTMLElement;
  readonly contentDOM: HTMLElement;

  constructor(node: Node) {
    const pre = document.createElement("pre");
    pre.className = "md-code-block md-code-block-plain";
    const language = languageOf(node);
    if (language) pre.dataset.language = language;
    const code = document.createElement("code");
    pre.append(code);
    this.dom = pre;
    this.contentDOM = code;
  }

  update(node: Node): boolean {
    if (node.type !== schema.nodes.code_block) return false;
    const language = languageOf(node);
    if (language) (this.dom as HTMLElement).dataset.language = language;
    else delete (this.dom as HTMLElement).dataset.language;
    return true;
  }
}

/** Arrow-into-code-block handling for the *outer* editor: when the
 *  caret is at a textblock edge and the neighbor is a code block,
 *  move the selection inside it (the node view's `setSelection`
 *  then focuses the embedded editor). */
function arrowHandler(
  dir: "left" | "right" | "up" | "down",
): Command {
  return (state: EditorState, dispatch, view) => {
    if (!view || !state.selection.empty || !view.endOfTextblock(dir)) {
      return false;
    }
    const side = dir === "left" || dir === "up" ? -1 : 1;
    const { $head } = state.selection;
    const border = side > 0 ? $head.after() : $head.before();
    if (border < 0 || border > state.doc.content.size) return false;
    const next = Selection.near(state.doc.resolve(border), side);
    if (!(next instanceof TextSelection)) return false;
    if (next.$head.parent.type !== schema.nodes.code_block) return false;
    if (dispatch) {
      dispatch(state.tr.setSelection(next).scrollIntoView());
    }
    return true;
  };
}

/** The code-block plugin bundle: node views + boundary keymap. */
export function codeBlocks(): Plugin[] {
  return [
    new Plugin({
      props: {
        nodeViews: {
          code_block(node, view, getPos) {
            // Decided per node view, at draw time: pages that
            // install <tonk-code> (its bundle lazy-loads language
            // chunks) get embedded editors; others keep a plain,
            // fully editable <pre>.
            if (customElements.get("tonk-code")) {
              return new TonkCodeBlockView(node, view, getPos);
            }
            return new PlainCodeBlockView(node);
          },
        },
      },
    }),
    keymap({
      ArrowLeft: arrowHandler("left"),
      ArrowRight: arrowHandler("right"),
      ArrowUp: arrowHandler("up"),
      ArrowDown: arrowHandler("down"),
    }),
  ];
}
