# dialog-reactor

Tonk's layer over [dialog](https://github.com/dialog-db/dialog-db) branches.
It caches repository and branch handles with the dialog stack opened over each,
runs queries (one-shot and live), commits transactions through the stack, and
streams a stack subscription's results to each connected client as tonk's wire
frames.

The [`Reactor`] sits between an application and the raw dialog repository/branch
API. Effects are described as chains and executed with `.perform(&env)`, matching
dialog's command/perform pattern: the reactor itself owns no operator and instead each
effect takes one at perform time.

```rust
use dialog_reactor::Reactor;

let reactor = Reactor::new(profile);

// One-shot read (no subscription registered):
let rows = reactor
    .repository("main")
    .branch("main")
    .query(concept_query)
    .perform(&operator)
    .await?;

// Mutate through the branch's stack, which schedules a poll of its
// subscriptions; `reactor.run_scheduled_polls(&operator)` settles it
// and sends each client what changed:
reactor
    .repository("main")
    .branch("main")
    .transaction()
    .assert(changes)
    .commit()
    .perform(&operator)
    .await?;
```

## What it provides

- **Cached handles.** First reference to a repo/branch opens it; later references
  reuse the cached handle, so the load+open cost is paid once per lifetime. The
  cached branch carries a warm content-addressed node cache, which pays off across
  repeated traversals (directory rows, nested renders).
- **Query effects.** [`QueryEffect`] (`branch.query(q)`) reads once through the
  stack and returns projected [`Conclusion`]s. [`Subscribe`] (`branch.subscribe(q)`)
  gives a client a reader of the stack's subscription to the query (dialog's
  `Stack::subscribe`: one evaluation per query, shared by its readers, held while a
  reader holds it). The reactor turns what the stack delivers into the client's
  frames, a snapshot first and deltas after, outermost display first; dropping the
  client's `Subscriber` releases its reader.
- **Transactions.** `branch.transaction().assert(…)/.retract(…).commit()` commits
  as one invocation through the stack, whose report's effect runs the stack's
  providers over the commands it landed.
- **Pull / push / export / import** chains over the same branch surface.
- **Commands.** `Decode`, `Migrated` and the `control:` tick rows the worker's
  stack providers decode a landed command with.

## Design

See [`reactor-spec.md`](./reactor-spec.md) for the original rationale: the chain
model and the caching invariants. Its subscription registry, query hash and poll
scheduling are dialog's stack subscriptions now.

> This crate eventually should move into the dialog-db repository hence it `dialog-` prefix.
