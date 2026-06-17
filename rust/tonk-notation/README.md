# tonk-notation

A pure parser for Tonk's asserted notation. It turns a YAML document into a
typed [`Syntax`] tree plus [`lsp_types::Diagnostic`]s, with no I/O and no name
resolution.

This crate sits at the bottom of the Tonk pipeline: notation source ->
`tonk-notation` parse -> `Syntax` tree -> [`tonk-analyzer`/`tonk-schema`]. The
parser captures the surface shape (what the user typed) and reports structural
problems; resolving names against a branch, deriving entity URIs, and building
queries and transactions all happen later, in the analyzer. It depends only on
`saphyr`/`saphyr-parser` (YAML), `lsp-types`, and `serde`, so it builds for both
native and `wasm32`.

## What it parses

Asserted notation is YAML. The document root is a mapping of `head: body`
entries, parsed in source order (duplicate heads are preserved, not collapsed,
so one logical query can span several blocks).

Each top-level entry becomes an [`Expression`], distinguished by the head's
trailing `!`:

- `head:` (no `!`) is a `Query` (reads facts matching the body's pattern).
- `head!:` is a `Claim` (writes facts of the head's concept). A claim wraps its
  application in an [`Effectful`] envelope that carries the optional `&anchor`
  written between the head's `:` and the body.

`rule!:` is not a separate variant. It is a `Claim` over the built-in `rule`
concept whose body fields (`assert!:`/`retract!:`/`when:`/`unless:`/
`description:`) the analyzer lifts into a rule mutation. Inside a rule body,
`when:`/`unless:` values are parsed as typed [`Premise`] lists
(`{ assert: <concept>, where: { … } }`).

Heads carry no inline binding. A head is classified by its lexical shape into a
[`HeadName`]:

- `Concept` for a bare identifier (`person`),
- `Claim` for a reverse-dotted domain (`xyz.tonk`),
- `Uri` for a scheme-prefixed URI or attribute identifier (`db:concept`,
  `id:person`, `did:key:…`, `xyz.tonk/name`).

Body field values become a [`FieldValue`]: a `Literal` [`Scalar`] (quoted or
typed YAML scalar), a `?var` `Variable`, a `_` `Blank` (matches any value in a
query, retracts the field in a claim), a bare-lowercase `Symbol` (name-table
reference), a scheme/attribute `Uri`, a `Nested` mapping, or a `Premises` list.
Quotes are load-bearing: a quoted or block scalar is always a string literal,
while a plain lowercase scalar classifies as a symbol.

The parser is permissive. A malformed expression yields a diagnostic and the
rest of the document still parses, so an editor can underline several problems
at once. YAML aliases (`*name`) are rejected (use a `&anchor` plus the bare
symbol, or a URI). Sequence-valued fields outside `when:`/`unless:` are rejected
in favor of repeated assertions.

## Types and entry points

`parse::parse(text: &str) -> Parsed` is the entry point. [`Parsed`] holds an
`Option<Syntax>` (present once the document reached the bottom) and the
`Vec<Diagnostic>` raised along the way. Every node carries an `lsp_types::Range`
so consumers can attach diagnostics to the source token they came from;
diagnostic ranges are clamped to stay inside the document.

The `Syntax` tree (in [`src/syntax.rs`](src/syntax.rs)):

- [`Syntax`]: the whole document: `expressions: Vec<Expression>` in source
  order, plus a document-covering `range`.
- [`Expression`]: `Query(Application)` or `Claim(Effectful<Application>)`.
- [`Application`]: the shared shape: a `predicate: Predicate` plus
  `fields: Vec<Field>`. Whether it reads or writes is decided by the wrapping
  `Expression` variant, not by the application itself.
- [`Effectful<T>`]: the `!` marker as a wrapper, holding the optional `&anchor`.
- [`Predicate`] / [`HeadName`], [`Field`] / [`FieldValue`], [`Scalar`],
  [`Premise`], [`Anchor`], and [`Spanned<T>`].

[`diagnostics::document_diagnostics(text) -> Vec<Diagnostic>`](src/diagnostics.rs)
is a stable wrapper over `parse` for language-server / lint callers (today it
returns the parse diagnostics; future validation passes compose here). The
module also exports the LSP `NOTATION_LANGUAGE_ID` and `SERVER_INFO` metadata.

## Modules

- [`parse`](src/parse.rs): the YAML walker (saphyr events -> `Syntax`),
  including a duplicate-preserving document loader and source-side `&anchor`
  recovery (saphyr exposes anchors only as numeric ids, so the name is scanned
  from the source).
- [`syntax`](src/syntax.rs): the typed tree.
- [`diagnostics`](src/diagnostics.rs): the top-level entry point and LSP
  metadata.

See [`guide.md`](guide.md) for the user-facing notation reference.
