# tonk-notation — design notes

Pure parser for the tonk asserted-notation DSL. See
[`guide.md`](guide.md) for the user-facing reference.

## Crate layout

- **`syntax.rs`** — typed AST: `Syntax → Vec<Expression>`, with
  `Expression = Query | Assertion | Retraction`. Each expression
  carries a `Head` (name + effect + binding) and either a body of
  `Vec<Field>` or, for retractions, just the head. Every node
  carries an `lsp_types::Range` so consumers can attach
  diagnostics to the source token.
- **`parse.rs`** — saphyr-backed YAML walker producing
  `Parsed { syntax: Option<Syntax>, diagnostics: Vec<Diagnostic> }`.
  Partial-parse semantics: a malformed expression yields a
  diagnostic and the rest of the document still produces a
  `Syntax`.
- **`diagnostics.rs`** — `document_diagnostics(text) = parse(text).diagnostics`,
  the entry point the language server uses.

## Why diagnostics, not errors

The parser never returns `Result`. Even a hard structural failure
(non-mapping root) produces a `Parsed` with `syntax: None` and a
diagnostic. The language server displays *all* problems, not just
the first.

## Why the AST owns its strings

Saphyr borrows from the input buffer (`MarkedYaml<'input>`); we
don't. The parser allocates owned `String`s into the AST so
callers can keep a `Syntax` around past the input's lifetime —
worker route handlers in particular drop the request body before
commit.

## Span policy

Real saphyr spans, converted to `lsp_types::Range` via
`position_at` / `range_of`. Zero-width spans get widened by one
column so editors render visible squiggles. Within the head
string (which encodes name + binding), sub-spans are not
extracted today — both the name and binding ranges point at the
whole key. Editors still highlight the right line.

## Head parsing

The level-1 YAML key is split on the first ASCII whitespace:

- Token before whitespace = name (with optional trailing `!` for
  effect mode).
- Everything after = binding text. Empty → `Anonymous`. Starts
  with `?` → `Variable`. Contains `:` → `Uri`. Else → `Bookmark`.

This lets URI bindings (`person! did:key:zX:`) work without a
sigil and avoids ambiguity around the trailing `:` that ends every
YAML key.

## Field-value classification

A string scalar becomes one of:

- `Blank` if exactly `_`
- `Variable(name)` if `?name`
- `Reference(Bookmark(name))` if `.name`
- `Reference(Uri(s))` if it contains `:`
- `Literal(String(s))` otherwise

The `.` sigil disambiguates bookmark references from literal
strings. Numeric / boolean / null saphyr scalars become
`Literal(Integer | Float | Boolean | Null)` directly.

Sequences are rejected with a diagnostic — the notation has no
representation for cardinality-many writes (use repeated
assertions).

## What's intentionally not here

- **Resolution.** Bookmark refs, URI refs, concept lookups all
  stay as references in the AST. The analyzer in `tonk-schema`
  does the resolving against whatever store it has access to.
- **Identity derivation.** No entity-URI computation. The AST
  only knows what the user wrote.
- **Dialog types.** This crate has no dialog-* dependencies.
  Everything that mentions `Entity`, `AttributeDescriptor`, etc.
  lives in `tonk-schema`.

## Consumers

- `tonk-language-server` — via `document_diagnostics` for editor
  squiggles.
- `tonk-schema::analyze` — consumes `Syntax` to produce an
  `Analysis` (queries + planned transactions).

The split is what lets a future LSP completion backend run its
own resolver against the same AST without re-parsing.
