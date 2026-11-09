// The core of the library's `<code-editor>`
// (rust/tonk-core/assets/library/code-editor.yaml).
//
// `<code-editor>` is branch data, so it has no URL to import chunks
// from: `scripts/build.mjs` builds this module into one expression, the
// element's `core` method, which evaluates to these exports.
//
// No language is bundled here. A language is a `code-editor/language`
// record in the space whose `source` is a pack: the text of a function
// `(shared) => LanguageSupport`. `shared` is the `shared` map below, the
// CodeMirror modules of THIS core, keyed by package name. A pack must
// reach them through it rather than bundle its own copies: CodeMirror
// checks extensions by `instanceof`, so a second `@codemirror/state`
// makes the pack's extensions unrecognized ("Unrecognized extension
// value in extension set"). The build gives every pack it emits that
// shape; a pack built anywhere else needs the same.
//
// Nothing here talks to a language server: the LSP client stays with
// `<tonk-code>`, which the host bundles with its provider.

import * as state from "@codemirror/state";
import * as view from "@codemirror/view";
import * as language from "@codemirror/language";
import * as commands from "@codemirror/commands";
import * as autocomplete from "@codemirror/autocomplete";
import * as lint from "@codemirror/lint";
import * as common from "@lezer/common";
import * as highlight from "@lezer/highlight";
import * as lr from "@lezer/lr";

export { createEditor, SHADOW_STYLESHEET } from "./editor";

/** The modules a pack shares with this core, by package name. Only
 *  these: they are the ones that carry the identities CodeMirror checks
 *  (`Facet`, `StateField`, `NodeProp`, the highlight tags). A pack
 *  bundles anything else it uses, so this list is the whole contract. */
export const shared: Record<string, unknown> = {
  "@codemirror/state": state,
  "@codemirror/view": view,
  "@codemirror/language": language,
  "@codemirror/commands": commands,
  "@codemirror/autocomplete": autocomplete,
  "@codemirror/lint": lint,
  "@lezer/common": common,
  "@lezer/highlight": highlight,
  "@lezer/lr": lr,
};

/** Packs evaluated in this realm, by source text: two blocks in the same
 *  language evaluate it once, and an edited record is a new source. */
const packs = new Map<string, Promise<state.Extension>>();

/** Evaluate a pack's `source` against this core.
 *
 *  Through a blob module rather than `eval` or `Function`: a page that
 *  forbids string evaluation can still allow `blob:` scripts, which is
 *  what the host already relies on to load `<tonk-code>`'s chunks in a
 *  sealed guest. A failure is not cached, so a corrected record loads. */
export function loadPack(source: string): Promise<state.Extension> {
  let pending = packs.get(source);
  if (!pending) {
    const url = URL.createObjectURL(
      new Blob([`export default (${source}\n);\n`], { type: "text/javascript" }),
    );
    pending = import(/* @vite-ignore */ url)
      .then((module) => (module.default as (shared: Record<string, unknown>) => state.Extension)(shared))
      .catch((error) => {
        packs.delete(source);
        throw error;
      })
      .finally(() => URL.revokeObjectURL(url));
    packs.set(source, pending);
  }
  return pending;
}
