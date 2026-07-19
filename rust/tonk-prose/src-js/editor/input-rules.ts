// Block-structure input rules — the conversions the reparse loop
// (reparse.ts) cannot do, because they change the *tree around* the
// textblock rather than the textblock itself: wrapping into
// blockquotes and lists, switching to fenced code blocks, and the
// horizontal-rule replacement.
//
// Deliberately absent: heading and inline-mark rules (`**bold**`,
// `` `code` ``, `[text](url)`, `# `). Those are the reparse loop's
// job — markers are literal text there, and a synchronous rule that
// applied the mark *without* the marker text would break the
// "textContent is the markdown source" invariant the loop depends
// on (the loop would promptly strip the mark again).
//
// Every conversion here is a single history step; `undoInputRule`
// (bound to Backspace in keymap.ts) restores the literal text —
// the same escape hatch Typora offers.

import {
  InputRule,
  inputRules,
  textblockTypeInputRule,
} from "prosemirror-inputrules";
import type { NodeType, Schema } from "prosemirror-model";
import type { Plugin } from "prosemirror-state";

/** "```lang " (trailing space) → code block. A bare "```" must NOT
 *  convert here — it would fire before a language can be typed —
 *  so the Enter-after-fence path lives in keymap.ts instead (input
 *  rules never see Enter; allusion forked the plugin for this, a
 *  keymap binding is the unforked way). */
function codeBlockRule(nodeType: NodeType): InputRule {
  return textblockTypeInputRule(
    /^```([a-zA-Z][\w+#-]*)\s$/,
    nodeType,
    (match) => ({ params: match[1] }),
  );
}

/** `---`, `***` or `___` alone on a line → horizontal rule. */
function horizontalRuleRule(nodeType: NodeType): InputRule {
  return new InputRule(/^(?:---|\*\*\*|___)$/, (state, _match, start, end) =>
    state.tr.replaceRangeWith(start, end, nodeType.create()),
  );
}

export function buildInputRules(schema: Schema): Plugin {
  return inputRules({
    rules: [
      // No blockquote / bullet / ordered-list rules: `> `, `- `, `1. `
      // all convert through the reparse loop like `# ` and every other
      // block prefix, so the marker text is preserved and the whole-
      // wrapper reparse (reparse.ts) derives the structure from it. A
      // synchronous wrapping rule instead created a MARKERLESS list or
      // quote, which the reparse then read as un-prefixed and lifted
      // right back out — the structure fought the caret and vanished as
      // soon as the next character was typed.
      codeBlockRule(schema.nodes.code_block),
      horizontalRuleRule(schema.nodes.horizontal_rule),
      // No image rule: images are expanded-image source text
      // (markup.ts), so typing `![alt](src)` converts through the
      // reparse loop like every other inline syntax.
    ],
  });
}
