import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { parseHTML } from "linkedom";
import { parseMarkdown } from "./markdown";
import { headingSwitcher, type Candidate, type Suggestion } from "./heading-switcher";

const library: Candidate[] = [{ title: "Scratch", href: "notebook/id:a" }];

/** A live view with the switcher, plus what it reported. */
function editor(doc: string, candidates: Candidate[] = library) {
  const dom = parseHTML("<!doctype html><html><body></body></html>");
  const saved = { document: globalThis.document, window: globalThis.window };
  Object.defineProperty(globalThis, "document", { value: dom.document, configurable: true });
  Object.defineProperty(globalThis, "window", { value: dom.window ?? dom, configurable: true });

  const seen: { rows: Suggestion[] | null; active: number }[] = [];
  const opened: Candidate[] = [];
  const created: string[] = [];
  const view = new EditorView(dom.document.createElement("div"), {
    state: EditorState.create({
      doc: parseMarkdown(doc),
      plugins: headingSwitcher({
        candidates: () => candidates,
        onOpen: (c) => opened.push(c),
        onCreate: (t) => created.push(t),
        onSuggest: (rows, active) => seen.push({ rows, active }),
      }),
    }),
  });
  const restore = () => {
    view.destroy();
    Object.defineProperty(globalThis, "document", { value: saved.document, configurable: true });
    Object.defineProperty(globalThis, "window", { value: saved.window, configurable: true });
  };
  return { view, seen, opened, created, restore, doc: dom.document };
}

/** Put the caret in the heading, after the `# ` marker. */
function caretInHeading(view: EditorView) {
  const { doc } = view.state;
  const end = 1 + doc.firstChild!.content.size;
  view.dispatch(view.state.tr.setSelection(TextSelection.create(doc, end)));
}

/** Type `text` at the caret. */
function type(view: EditorView, text: string) {
  view.dispatch(view.state.tr.insertText(text));
}

test("typing in the heading updates the suggestions", () => {
  const e = editor("# ");
  try {
    caretInHeading(e.view);
    type(e.view, "Scr");
    const last = e.seen[e.seen.length - 1];
    assert.ok(last?.rows, "a query reports rows");
    assert.ok(
      last.rows!.some((r) => r.title === "Scratch" && !r.create),
      `the match is offered: ${JSON.stringify(last.rows)}`,
    );
    assert.ok(
      last.rows!.some((r) => r.create),
      "and so is the create row",
    );
  } finally {
    e.restore();
  }
});

test("an empty space still offers the create row", () => {
  const e = editor("# ", []);
  try {
    caretInHeading(e.view);
    type(e.view, "Fresh");
    const last = e.seen[e.seen.length - 1];
    assert.ok(
      last?.rows?.some((r) => r.create && r.title === "Fresh"),
      `with nothing to match, creating is still on offer: ${JSON.stringify(last?.rows)}`,
    );
  } finally {
    e.restore();
  }
});

/** Press a key through the view's own handler, as the browser would. */
function press(view: EditorView, key: string): boolean {
  return view.someProp("handleKeyDown", (f) => f(view, { key } as KeyboardEvent)) ?? false;
}

test("Enter on an exact title opens that notebook", () => {
  const e = editor("# ");
  try {
    caretInHeading(e.view);
    type(e.view, "Scratch");
    assert.equal(press(e.view, "Enter"), true, "the switcher claims Enter");
    assert.deepEqual(
      e.opened.map((c) => c.href),
      ["notebook/id:a"],
      "and opens the match rather than creating a duplicate",
    );
    assert.deepEqual(e.created, [], "nothing is created");
  } finally {
    e.restore();
  }
});

test("Enter on a new name creates it", () => {
  const e = editor("# ", []);
  try {
    caretInHeading(e.view);
    type(e.view, "Groceries");
    assert.equal(press(e.view, "Enter"), true);
    assert.deepEqual(e.created, ["Groceries"], "the typed name is created");
    assert.deepEqual(e.opened, [], "nothing is opened");
  } finally {
    e.restore();
  }
});

test("the arrows move the highlight", () => {
  const e = editor("# ");
  try {
    caretInHeading(e.view);
    type(e.view, "Scr");
    const before = e.seen[e.seen.length - 1].active;
    press(e.view, "ArrowDown");
    const after = e.seen[e.seen.length - 1].active;
    assert.notEqual(after, before, "ArrowDown moves it");
  } finally {
    e.restore();
  }
});

test("Enter outside the heading is the editor's", () => {
  const e = editor("# Notes\n\nbody");
  try {
    // Caret in the body, not the heading.
    const pos = e.view.state.doc.content.size - 1;
    e.view.dispatch(
      e.view.state.tr.setSelection(TextSelection.create(e.view.state.doc, pos)),
    );
    assert.equal(press(e.view, "Enter"), false, "the switcher does not claim it");
  } finally {
    e.restore();
  }
});
