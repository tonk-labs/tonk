# dialog-reactor

A reactive layer over [dialog](https://github.com/dialog-db/dialog-db) branches.
It caches repository and branch handles, runs queries (one-shot and live), commits
transactions, and fans changed query results out to subscribers — all without
binding to any particular host (service worker, CLI, server).

The [`Reactor`] sits between an application and the raw dialog repository/branch
API. Effects are described as chains and executed with `.perform(&env)`, matching
dialog's command/perform pattern: the reactor itself owns no operator — every
effect takes one at perform time.

```rust
use dialog_reactor::Reactor;

let reactor = Reactor::new(profile);

// One-shot read (no subscription registered):
let rows = reactor
    .repository("main").branch("main")
    .query(concept_query)
    .perform(&operator).await?;

// Mutate; every subscription on the branch re-evaluates and
// broadcasts changed results automatically:
reactor
    .repository("main").branch("main")
    .transaction()
    .assert(changes)
    .commit()
    .perform(&operator).await?;
```

## What it provides

- **Cached handles.** First reference to a repo/branch opens it; later references
  reuse the cached handle, so the load+open cost is paid once per lifetime. The
  cached branch carries a warm content-addressed node cache, which pays off across
  repeated traversals (directory rows, nested renders).
- **Query effects.** [`QueryEffect`] (`branch.query(q)`) reads once and returns
  projected [`Conclusion`]s. [`Subscribe`] (`branch.subscribe(q)`) opens or
  attaches to a standing subscription whose first message is the current snapshot
  and whose subsequent messages are change broadcasts, deduplicated by hash.
- **Transactions.** `branch.transaction().assert(…)/.retract(…).commit()` applies
  atomically and re-polls every subscription on the branch so changed results fan
  out without callers remembering to.
- **Pull / push / export / import** chains over the same branch surface.
- **Commands.** The transient-concept command machinery
  ([`CommandRegistry`], [`CommandHandler`], [`TypedCommand`]) runs registered
  commands after a commit. It is generic over the `Env` the commands execute
  against (`CommandRegistry<Env>`), so each consumer instantiates it with its own
  application environment; a command is registrable only when `Env: Provider<C>`,
  making capability a compile-time gate.

## Design

See [`reactor-spec.md`](./reactor-spec.md) for the full rationale — the chain
model, the subscription/broadcast lifecycle, and the caching invariants.

## Consumers

- `tonk-worker` — the service-worker HTTP shell; routes mutate and query branches
  through the reactor and stream subscription frames over SSE.
- `slide` — the headless CLI; uses the reactor as its single branch-access layer.

This crate is destined to move into the dialog-db repository; it deliberately
depends only on `dialog-*` crates plus `tonk-schema`/`tonk-common`/`tonk-evaluator`
and carries no HTTP, DOM, or wasm-bindgen coupling.
