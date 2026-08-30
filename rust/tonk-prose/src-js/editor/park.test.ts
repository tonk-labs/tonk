import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState } from "prosemirror-state";
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
