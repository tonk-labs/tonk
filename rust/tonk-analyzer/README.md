# tonk-analyzer

Analyzer and `Analysis<T>` tree IR for Tonk notation documents.

This crate turns a parsed [`tonk_notation::Syntax`] tree into a typed
`Analysis<Syntax>` tree: it resolves every name against a [`tonk_schema`] schema
and mints the synthesized queries and applications that downstream evaluation
runs. It sits above `tonk-schema` (it resolves notation against the schema's
concepts and attributes) and below `tonk-evaluator` (which drives the analyzer's
output against a repository).

## The `Analysis<T>` tree

`analyze`'s output is structurally an `Analysis<T>` — each parsed syntax node
paired with the analysis computed for it, threaded through one generic:

```text
Analysis<Syntax>           .analysis = DocumentAnalysis
  Analysis<Expression>     .analysis = ExpressionAnalysis (per variant)
    Analysis<Application>    .analysis = QueryNodeAnalysis   (queries)
    Analysis<Application>    .analysis = AssertionAnalysis   (claims)
```

The tree mirrors the document: one `Analysis<Expression>` per top-level
expression, in document order. Claims and queries share the same syntactic shape
(`Application`) — only the wrapping `Expression` variant distinguishes them.

A `rule!:` is a claim whose predicate is the built-in `rule` concept; when the
analyzer recognizes it, the produced `AssertionAnalysis` carries an `effect`
payload that lowers to a `Statement::InstallEffect` rather than a per-field claim.

## Two phases: resolve, then expand

Analysis runs in two named sub-phases (see `analysis-spec.md` next to the crate
and `plan/runtime.md` for the full design):

- **resolve** — walk the document and bind every concept / attribute reference
  through the `Scope`'s resolution chain (which calls into
  `tonk_schema::resolution` with the per-execution `env`). Record content-derived
  entities into `declarations` (anchor-form heads) and `variables` (variable-form
  heads), and scan for diagnostics. For `attribute!` / `concept!` heads the body
  is parsed here, so the descriptor's content-addressed entity is known up front.
  Output keeps the source shape.
- **expand** — lower notation sugar into kernel-shaped claims: a domain predicate
  becomes an anonymous concept, an `&anchor` pairs with a built-in `Name` assert,
  an omitted `this:` is injected as `id:<body-digest>`. This builds the query
  `Application`s, the mutation `Statement`s, the `rule!:`-to-`InstallEffect` lift,
  and the implicit snapshot queries.

## Modules

- [`analysis`](src/analysis.rs) — the `Analysis<T>` tree IR and its per-node
  analysis payloads.
- [`analyzer`](src/analyzer.rs) — the two-phase driver (`resolve` then `expand`),
  with the per-concern logic split across [`analyzer/`](src/analyzer/):
  `scope`, `declaration`, `assertion`, `query`, `rule`, `formula`, `constraint`,
  `field`, `graph`, `scan`, `error`.

## Consumers

`tonk-evaluator` runs the analyzer under the hood in its `compile` / `evaluate`
chain; the language server uses the `Analysis<T>` tree and its diagnostics
directly to power editor feedback.
