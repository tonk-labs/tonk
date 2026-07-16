// Block-structure input rules — the conversions the reparse loop
// (reparse.ts) cannot do, because they change the *tree around* the
// textblock rather than the textblock itself: wrapping into
// blockquotes and lists, switching to fenced code blocks, and the
// atomic replacements (horizontal rule, image).
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
  wrappingInputRule,
} from "prosemirror-inputrules";
import type { NodeType, Schema } from "prosemirror-model";
import type { Plugin } from "prosemirror-state";

/** `> ` at the start of a textblock → blockquote. */
function blockquoteRule(nodeType: NodeType): InputRule {
  return wrappingInputRule(/^\s*>\s$/, nodeType);
}

/** `- `, `+ `, `* ` at the start of a textblock → bullet list. */
function bulletListRule(nodeType: NodeType): InputRule {
  return wrappingInputRule(/^\s*([-+*])\s$/, nodeType);
}

/** `1. ` at the start of a textblock → ordered list, numbering
 *  carried into the list's `order` attribute. */
function orderedListRule(nodeType: NodeType): InputRule {
  return wrappingInputRule(
    /^(\d+)\.\s$/,
    nodeType,
    (match) => ({ order: +match[1] }),
    (match, node) =>
      node.childCount + (node.attrs.order as number) === +match[1],
  );
}

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

/** `![alt](src)` → image node. Images are atoms (no faithful text
 *  form), so the reparse loop deliberately never creates them —
 *  this rule is the only typing path. */
function imageRule(nodeType: NodeType): InputRule {
  return new InputRule(
    /!\[([^\]]*)\]\(([^)\s]+)(?:\s+"([^"]*)")?\)$/,
    (state, match, start, end) => {
      const [, alt, src, title] = match;
      if (!src) return null;
      return state.tr.replaceRangeWith(
        start,
        end,
        nodeType.create({ src, alt: alt || null, title: title || null }),
      );
    },
  );
}

export function buildInputRules(schema: Schema): Plugin {
  return inputRules({
    rules: [
      blockquoteRule(schema.nodes.blockquote),
      bulletListRule(schema.nodes.bullet_list),
      orderedListRule(schema.nodes.ordered_list),
      codeBlockRule(schema.nodes.code_block),
      horizontalRuleRule(schema.nodes.horizontal_rule),
      imageRule(schema.nodes.image),
    ],
  });
}
