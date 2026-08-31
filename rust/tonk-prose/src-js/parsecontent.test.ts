import { test } from "node:test";
import assert from "node:assert/strict";
import { parseContent } from "./editor/content";

test("a bare heading marker survives parseContent", () => {
  // The index's starting document is `"# "`. If the envelope parser
  // trims it, the editor mounts on an empty paragraph: no heading, so no
  // switcher and no visible marker.
  assert.equal(parseContent("# ").value, "# ");
});
