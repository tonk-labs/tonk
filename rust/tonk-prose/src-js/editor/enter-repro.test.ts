import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { parseHTML } from "linkedom";
import { parseMarkdown } from "./markdown";
import { headingSwitcher, type Candidate, type Suggestion } from "./heading-switcher";

/** Reproduce the browser sequence exactly: mount, click in, type, Enter. */
test("Enter after typing into a mounted editor reaches onCreate", () => {
  const dom = parseHTML("<!doctype html><html><body></body></html>");
  const saved = { document: globalThis.document, window: globalThis.window };
  Object.defineProperty(globalThis, "document", { value: dom.document, configurable: true });
  Object.defineProperty(globalThis, "window", { value: dom.window ?? dom, configurable: true });
  const created: string[] = [];
  const suggested: (Suggestion[] | null)[] = [];
  const candidates: Candidate[] = [{ title: "Scratch", href: "notebook/id:a" }];
  const view = new EditorView(dom.document.createElement("div"), {
    state: EditorState.create({
      doc: parseMarkdown("# "),
      plugins: headingSwitcher({
        candidates: () => candidates,
        onOpen: () => {},
        onCreate: (t) => created.push(t),
        onSuggest: (rows) => suggested.push(rows),
      }),
    }),
  });
  try {
    // The park runs in a microtask in the browser; here the caret starts at
    // 0, which is what a click into an empty heading also gives.
    const end = 1 + view.state.doc.firstChild!.content.size;
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, end)));
    view.dispatch(view.state.tr.insertText("Kale"));

    const rows = suggested[suggested.length - 1];
    assert.ok(rows && rows.length > 0, `the panel has rows: ${JSON.stringify(rows)}`);

    const handled = view.someProp("handleKeyDown", (f) =>
      f(view, { key: "Enter" } as KeyboardEvent),
    );
    assert.equal(handled, true, "the switcher claims Enter");
    assert.deepEqual(created, ["Kale"], "and onCreate fires with the typed name");
  } finally {
    view.destroy();
    Object.defineProperty(globalThis, "document", { value: saved.document, configurable: true });
    Object.defineProperty(globalThis, "window", { value: saved.window, configurable: true });
  }
});
