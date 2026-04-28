# tonk-notation — design notes

Pure parser for asserted notation. Owns the typed AST (`Syntax`)
plus YAML and JSON entry points; emits diagnostics anchored to
source spans.

## What's in here

- **`syntax.rs`** — typed AST: `Syntax → Statement → Subject + Vec<Context>`,
  with `Context = Domain | Attribute | Concept | UserConcept`.
  Concept fields use `Reference = Bookmark | Uri | Inline`. Every
  node carries an `lsp_types::Range` so consumers can attach
  diagnostics to the source token they came from.
- **`parse.rs`** — two entry points, both produce `Parsed { syntax: Option<Syntax>, diagnostics }`:
  - `parse(text)` — saphyr-backed YAML walker with **partial-parse**
    semantics: a malformed statement gets a diagnostic, the rest
    of the document still produces a `Syntax`. Lets the language
    server underline several problems at once.
  - `parse_json(text)` — serde_json plus
    `Syntax::try_from(&serde_json::Value)`. JSON's strict syntax
    means structural errors abort the whole parse: `syntax: None`
    whenever any diagnostic was raised.
- **`diagnostics.rs`** — `document_diagnostics(text) = parse(text).diagnostics`,
  the entry point the language server already uses.

## Notation

Three-level subject/context/fields. Same shape carry's CLI uses.

```yaml
person-name:                 # subject (level 1)
  attribute:                 # context (level 2)
    the:         io.gozala.person/name
    as:          Text
    cardinality: one
```

- **Subject** is classified into `Bookmark` / `Uri` / `Anonymous` /
  `Variable` by lexical inspection (`:` → URI, `_` → Anonymous,
  `?…` → Variable, otherwise Bookmark). The parser does not
  resolve or reject any of them — that's the interpreter's call.
- **Context** is a `Domain` if its key contains `.`, the
  built-in `attribute` / `concept` if it's one of those, or a
  `UserConcept` otherwise. Acceptance is again deferred.
- **Fields** under a domain context are raw scalars or sequences
  (or maps for nested entities — currently unsupported by the
  interpreter, captured in the AST anyway). Fields under
  `concept:` are `Reference`s.

## Why diagnostics, not errors

The parser never returns `Result`. Even a hard structural failure
(non-mapping root) produces a `Parsed` with `syntax: None` and a
diagnostic. Two reasons:

1. The language server wants to display *all* problems, not just
   the first one. A document with three malformed statements
   shouldn't show one error and hide the others.
2. Surface-level differences between YAML and JSON shouldn't
   leak into the API. Both call sites get `Parsed`, both walk
   `diagnostics`, both treat `syntax: None` the same way.

## Why the AST owns its strings

Saphyr borrows from the input buffer (`MarkedYaml<'input>`); we
don't. The parser allocates owned `String`s into the AST so
callers can keep a `Syntax` around past the input's lifetime —
worker route handlers, in particular, drop the request body
before commit.

## Span policy

YAML path: real saphyr spans, converted to `lsp_types::Range`
via `position_at` / `range_of`. Zero-width spans get widened by
one column so editors render visible squiggles.

JSON path: serde_json values have no position info. Every range
is `Range::default()`, which renders as a document-level
annotation in LSP clients. Document this; don't fake it.

## Reserved-prefix rule

Earlier revisions of `shape.rs` flagged `dialog.*` domains as
errors per a draft RFC reservation. That was wrong — both carry
and our transact route legitimately write under `dialog.meta`,
`dialog.attribute`, `dialog.concept.*`. The rule is gone and
the test that asserted it (`dialog_prefix_is_no_longer_an_error`)
verifies it stays gone.

## What's intentionally not here

- **Resolution.** Bookmark refs, URI refs, user-concept lookups
  all stay as references in the AST. The interpreter does the
  resolving against whatever store it has access to.
- **Identity derivation.** No entity-URI computation. The AST
  only knows what the user wrote.
- **Dialog types.** This crate has no dialog-* dependencies.
  Everything that mentions `Entity`, `AttributeDescriptor`, etc.
  lives in `tonk-schema`.

## Consumer

Currently just `tonk-language-server` (via `document_diagnostics`)
and `tonk-schema::interpret` (via `parse` / `parse_json` →
`Syntax`). The split is what lets a future LSP completion
backend run its own resolver against the same AST without
re-parsing.
