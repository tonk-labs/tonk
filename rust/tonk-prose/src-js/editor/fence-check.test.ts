import { test } from "node:test";
import assert from "node:assert/strict";
import { parseMarkdown, serializeMarkdown } from "./markdown";

test("a fenced block keeps its language and content", () => {
  const src = "```dialog-yaml\nconcept:\n  this: id:x\n```";
  const doc = parseMarkdown(src);
  let found: {lang: unknown, text: string} | null = null;
  doc.descendants((node) => {
    if (node.type.name === "code_block") {
      found = { lang: node.attrs.params ?? node.attrs.language, text: node.textContent };
    }
    return true;
  });
  assert.ok(found, "the fence parses to a code_block node");
  assert.equal(found!.text, "concept:\n  this: id:x");
  assert.equal(found!.lang, "dialog-yaml", "language survives, which is what highlighting keys on");
  assert.equal(serializeMarkdown(doc), src);
});

test("a fence containing blank lines is one block", () => {
  const src = "```dialog-yaml\na:\n\nb:\n```";
  assert.equal(serializeMarkdown(parseMarkdown(src)), src);
});
