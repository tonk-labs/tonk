// Contract between the `<tonk-prose>` shell (`../index.ts`) and the
// lazily-loaded editor core (`./index.ts`). The shell imports ONLY
// types from this module — anything with a runtime footprint must
// stay out of it, or the shell chunk stops being tiny.

import type { EditorView } from "prosemirror-view";

/** Options the shell hands to `createEditor`. */
export type EditorOptions = {
  /** Initial document, as markdown source. */
  doc: string;
  /** Lock the editor (no caret, no edits; selection still works). */
  readOnly: boolean;
  /** Ghost text shown while the document is empty. */
  placeholder: string;
  /** Called after every user edit with the markdown serialization
   *  of the new document. Not called for programmatic writes. */
  onChange: (markdown: string) => void;
};

/** Live editor handle returned by `createEditor`. */
export interface ProseEditor {
  /** Serialize the current document to markdown. */
  getMarkdown(): string;
  /** Parse `markdown` and replace the document. Preserves undo
   *  history (the replacement is one undoable step). */
  setMarkdown(markdown: string): void;
  setReadOnly(readOnly: boolean): void;
  setPlaceholder(text: string): void;
  /** Move keyboard focus into the document. */
  focus(): void;
  /** Tear down the view and release resources. */
  destroy(): void;
  /** The underlying ProseMirror view. Power-user escape hatch. */
  readonly view: EditorView;
}

/** Shape of the dynamically-imported editor-core module. */
export type EditorModule = {
  createEditor(parent: HTMLElement, options: EditorOptions): ProseEditor;
};
