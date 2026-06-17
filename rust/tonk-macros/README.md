# tonk-macros

Compile-time macros for tonk.

This is a proc-macro crate (`proc-macro = true`). It lowers a self-contained
notation file into its embedded, typed form at build time, so the document is
parsed, analyzed, and lowered by the compiler rather than at runtime.

## `claim!`

`claim!("path.yaml")` reads a notation file at compile time, analyzes it against
its own definitions with no running system, and embeds the lowered result. It
expands to a `(TransactRequest, Vec<Rule>)`:

- a [`tonk_core::claim::TransactRequest`] reconstructed from embedded canonical
  DAG-JSON bytes (the document's concept claims), and
- a `Vec<tonk_schema::rule::Rule>`, one per `rule!:` install in the document
  (empty when there are none).

```rust
use std::sync::LazyLock;
use tonk_core::claim::TransactRequest;
use tonk_schema::rule::Rule;

static BOOTSTRAP: LazyLock<(TransactRequest, Vec<Rule>)> =
    LazyLock::new(|| tonk_macros::claim!("bootstrap.yaml"));
```

The argument is a path relative to the calling crate's `CARGO_MANIFEST_DIR`
(same convention as `include_str!`). The file is parsed and analyzed with no
branch: every reference must resolve against the document's own `concept!` /
`attribute!` / `&anchor` definitions (plus builtins). A reference that would
need a running system, a parse error, or an analysis error all become compile
errors.

Rules have no `TransactRequest` representation (the `Claim` wire cannot carry
`dialog.effect/*` triples), so each `rule!:` install is embedded as its
`(source, polarity)` and rebuilt at runtime via
[`tonk_core::effect::Effect::from_source`] plus `Rule::asserting`. The macro
also emits an `include_bytes!` of the source path so cargo records it as a build
dependency and re-runs when the document changes (a plain `std::fs` read inside
a proc-macro is invisible to cargo's dependency graph).

## Dependencies

`syn`, `quote`, and `proc-macro2` for the macro plumbing; `serde_ipld_dagjson`
to serialize the lowered request; and `tonk-notation`, `tonk-analyzer`, and
`tonk-core` to parse, analyze, and lower the document at build time.
