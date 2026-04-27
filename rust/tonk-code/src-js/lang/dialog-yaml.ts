// `dialog-yaml` language pack.
//
// Dialog notation is YAML at the syntax level — the difference is
// semantic (entity → context → fields hierarchy, reserved
// `dialog.*` namespace, `?var` unification, etc.). The semantic
// layer is enforced by the language server (`tonk-notation` →
// asserted-notation validators), not by the grammar.
//
// Phase 0 of this pack therefore *is* the YAML grammar, with no
// dialect-specific decorations. Future iterations will layer a
// `ViewPlugin` that walks the syntax tree and decorates DIDs,
// `?vars`, the `_` anonymous-entity sigil, and reserved-prefix
// domains so the editor visually communicates dialect-level
// semantics on top of the YAML colors.
//
// This file's role at the build level is to give the consumer a
// stable language id (`dialog-yaml`) regardless of how the
// underlying highlighting is composed — adding a parser-mixed
// dialect grammar later replaces the import below without
// changing the public id.

import { yaml } from "@codemirror/lang-yaml";

export default yaml();
