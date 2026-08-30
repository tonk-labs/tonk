import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { parseHTML } from "linkedom";
import { parseMarkdown } from "./markdown";
import { headingSwitcher } from "./heading-switcher";
import { schema } from "./schema";

/** The switcher's plugins, with inert hooks. */
function plugins() {
  return headingSwitcher({
    candidates: () => [],
    onOpen: () => {},
    onCreate: () => {},
    onSuggest: () => {},
  });
}

test("the switcher's plugins load into a state without throwing", () => {
  const state = EditorState.create({ doc: parseMarkdown("# "), plugins: plugins() });
  assert.equal(state.doc.firstChild?.type.name, "heading");
});

test("the marker text is inside the heading, so a caret at 1 is before it", () => {
  // Position 1 is the start of the heading's content, which is the marker.
  // The caret must land at 1 + marker length, or typing goes before the `#`.
  const doc = parseMarkdown("# ");
  const heading = doc.firstChild!;
  assert.equal(heading.textContent, "# ");
  assert.equal(1 + heading.content.size, 3, "the end of the heading's content");
});

test("constructing a real EditorView with the switcher does not throw", () => {
  // The state-only test above passes even when the plugin dispatches during
  // `new EditorView`: that fault only fires once a VIEW is constructed,
  // where `view()` runs before the constructor has bound `view` and a
  // dispatch re-enters `dispatchTransaction` ("Cannot access 'i' before
  // initialization"). Building the view is what catches it.
  // ProseMirror reads globals (`document`, `window`) rather than taking a
  // document, so the linkedom one has to be installed globally for the
  // length of the test.
  const dom = parseHTML("<!doctype html><html><body></body></html>");
  // `navigator` is a getter-only global in Node, so only document/window
  // are swapped; ProseMirror reads navigator but tolerates Node's own.
  const saved = {
    document: globalThis.document,
    window: globalThis.window,
  };
  Object.defineProperty(globalThis, "document", {
    value: dom.document,
    configurable: true,
  });
  Object.defineProperty(globalThis, "window", {
    value: dom.window ?? dom,
    configurable: true,
  });
  try {
    const parent = dom.document.createElement("div");
    const state = EditorState.create({ doc: parseMarkdown("# "), plugins: plugins() });
    const view = new EditorView(parent, { state });
    assert.equal(view.state.doc.firstChild?.type.name, "heading");
    view.destroy();
  } finally {
    Object.defineProperty(globalThis, "document", {
      value: saved.document,
      configurable: true,
    });
    Object.defineProperty(globalThis, "window", {
      value: saved.window,
      configurable: true,
    });
  }
});
