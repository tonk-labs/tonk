# tonk-evaluator

Evaluation of analyzed Tonk notation documents against a repository.

This crate sits at the top of the Tonk dependency graph
(`tonk-evaluator → tonk-analyzer → tonk-schema → tonk-core`). It takes the
analysis tree the analyzer produces from an asserted-notation document, drives it
against a branch — running the synthesized queries, staging mutations, firing
installed effects — and hands the caller a transaction ready to commit.

## The evaluate pipeline

Three chain handles hang off [`tonk_notation::Syntax`], each a nested prefix of
the next. Each is described, then run with `.perform(env)`:

```rust
use tonk_evaluator::evaluate::{SyntaxAnalyzeExt, SyntaxCompileExt, SyntaxEvaluateExt};

let analysis  = syntax.analyze(source).perform(env).await?;          // pure read -> Analysis
let compiled  = syntax.compile(source).perform(env).await?;          // -> Compiled (runnable ops)
let evaluated = syntax.evaluate(branch.transaction()).perform(env).await?; // -> Evaluated (changes staged)
```

- **`analyze`** takes a [`Source`] (anything `Into<Source>` — a `&Branch` or
  `&Transaction`) and yields an `Analysis`. Read-only.
- **`compile`** runs `analyze` under the hood and yields a `Compiled` handle over
  the resolved document's runnable operations.
- **`evaluate`** opens a transaction, compiles, runs the operations, and yields an
  `Evaluated` holding the transaction with the document's changes staged. It does
  **not** commit — committing is the caller's choice.

## Effects (the induce fixpoint)

`evaluate` does not fire inductive rules; that is a separate, explicit step so the
caller controls when it runs. The public surface is `TransactionExt::induce`,
which mirrors dialog's `Branch::commit(...)` chain pattern:

```rust
use tonk_evaluator::effects::TransactionExt;

let txn = branch.transaction()
    .assert(changes)
    .induce(transients)          // run the rule fixpoint over the overlay
    .perform(env).await?;
let revision = txn.commit().perform(env).await?;
```

`induce` runs the installed `rule!:` effects to a fixpoint against the
transaction's overlay (so rules see branch state *unioned* with the pending
writes of the same commit), retracts transient claims, and returns the
post-induction transaction. All reads go through `Transaction::query`, so no
`&Branch` is needed at the boundary.

## Modules

- [`evaluate`](src/evaluate.rs) — the analyze → compile → evaluate chain
  (`SyntaxAnalyzeExt` / `SyntaxCompileExt` / `SyntaxEvaluateExt`, `Compiled`,
  `Evaluated`).
- [`effects`](src/effects.rs) — `TransactionExt::induce` and the inductive-rule
  fixpoint (`Induce`, `InduceError`).
- [`effect_query`](src/effect_query.rs) — effect storage, lookup, and
  install-time validation (loading effects back from a branch, the V1
  transient-trigger requirement, writing effects into a transaction).

See `plan/effects.md` for the conceptual model behind the induce fixpoint.

## Consumers

`slide`, `tonk-worker`, and `dialog-reactor` (whose commit path runs the induce
fixpoint) all drive documents through this pipeline. Effects are an explicit
`.induce(...)` step rather than something `evaluate` does implicitly, so each
consumer decides when the fixpoint runs and when it commits.
