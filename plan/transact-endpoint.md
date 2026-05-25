# `/transact` endpoint — structured mutations with preserved transient classification

## Motivation

Today, every mutation reaches the worker through one of two paths:

1. `/api/repository/{repo}/branch/{branch}/claim/assert` and
   `/claim/retract` — accept raw EAV triples. The wire format has
   already collapsed concept-level structure into individual facts.
2. `/api/repository/{repo}/branch/{branch}/evaluate` — accepts an
   asserted-notation document (YAML-shaped). `tonk_schema::evaluate::run`
   parses, analyzes, queries, plans, and commits in one pass. The
   analyzer knows each statement's concept; the planner emits
   `RawClaim` triples; the transaction builder is the raw dialog one.

By the time either path reaches the dialog `Changes` batch, every
concept-level distinction is gone. Effects evaluation (see
[effects.md](./effects.md)) needs to know which assertions belong to
transient concepts so it can retract them inside the same transaction.
Recovering this from raw EAV at commit time means a query against the
schema for each touched attribute — wasted work given that the writer
already knew.

The fix is to introduce a structured wire path that carries concept-level
classification through to the reactor's transaction builder, and to route
both new callers and the existing `/evaluate` mutations through it.

## Shape

### Wire format (new module: `tonk-schema::transact`)

```rust
/// One mutation step. The wrapper carries enough context for the
/// reactor to know whether the resulting claims belong to a
/// transient concept without re-querying the schema.
pub enum Application {
    /// A claim that should land in durable storage and persist
    /// across timesteps (the default).
    Durable(ConceptApplication),
    /// A claim that exists only at the current timestep. The
    /// reactor will retract it inside the commit transaction so
    /// it never reaches durable storage, but it remains visible
    /// to effects' deductive saturation at this timestep.
    Transient(ConceptApplication),
}

/// A concept descriptor + the parameter bindings needed to
/// instantiate it. Parallel to the query-side `ConceptQuery`
/// shape: descriptor + parameters.
pub struct ConceptApplication {
    pub concept: ConceptDescriptor,
    pub parameters: Parameters,
}

pub enum Statement {
    Assert(Application),
    Retract(Application),
}

pub struct TransactRequest {
    pub statements: Vec<Statement>,
}
```

The `Transient` variant lives at `Application` rather than on
`ConceptDescriptor` because adding a `transient` field to the dialog
descriptor would require an upstream change. Keeping the classification
in the wrapper lets us validate the design end-to-end before pushing the
flag down into `ConceptDescriptor` (if we ever decide that's the right
home).

### Endpoint

```
POST /api/repository/{repo}/branch/{branch}/transact
POST /api/profile/branch/{branch}/transact
```

Body: `TransactRequest` (JSON). Response mirrors `/evaluate`'s
`CommitSummary` (revision before/after, claim count, committed flag).
No query-string `transact=false` parameter — `/transact` always
commits; the "dry-run" use case is `/query` + client-side projection.

### Reactor builder changes

`TransactionBuilder` gains typed application methods and a transient
bucket alongside the existing `Changes`:

```rust
pub struct TransactionBuilder<'a> {
    pub branch: BranchReference<'a>,
    /// Durable claims — flow into the dialog Changes batch.
    pub changes: Changes,
    /// Transient claims — kept aside so `retract_transients` can
    /// sweep them at the end of the fixpoint without re-querying.
    pub transients: Changes,
}

impl<'a> TransactionBuilder<'a> {
    pub fn assert(self, application: Application) -> Self { ... }
    pub fn retract(self, application: Application) -> Self { ... }
}
```

Each method plans the `Application` into `RawClaim` triples and routes
them into `changes` or `transients` based on the variant. The existing
raw `Statement`-typed entrypoints (`assert<S: Statement>(claim: S)`) stay
for now so `/claim/*` and any other low-level callers keep working; the
typed entrypoints take precedence for new code.

`Commit::perform` integrates both batches into the dialog `Transaction`
(so effects' deductive saturation can see transients via the overlay),
runs `evaluate_effects`, then `retract_transients` walks `self.transients`
and pushes a retract for each claim — no querying, just iteration over
known facts.

### `/evaluate` adaptation

`tonk_schema::evaluate::run` already has the concept on each
`analysis.mutate.statements` entry. The change is:

1. When projecting an analyzer `Statement::Assert(plan)` to a builder
   call, look at the concept's transient flag (carried through
   tonk-notation's analyzer state alongside the descriptor) and call
   `tx.assert(Application::Transient(...))` or
   `tx.assert(Application::Durable(...))` accordingly.
2. Reject documents that mix transient and durable concepts in ways
   the analyzer can prove are incoherent. The exact rule is TBD; the
   minimum is: don't silently flatten one to the other.
3. Use the reactor's `TransactionBuilder` (already in place via
   `BranchReference::transaction`), not raw `branch.transaction()`.
   This is the load-bearing change that finally routes notation-driven
   mutations through `Commit::perform` so effects fire.

## Evaluation model — per-round timestamps, batched commit

Effects are *inductive* rules: a body that holds at *t* produces a
head at *t+1*. Strict Dedalus would commit each step. Tonk doesn't,
because intermediate states between rounds aren't observable to
subscribers — only the final settled state matters. So we run the
fixpoint inside one commit and emit a single durable write.

### Per-round loop

Each round is a logical timestamp transition. Round *k* reads
state at *t+k*, fires the rules triggered by transients at *t+k*,
and produces facts at *t+k+1*.

1. Run all triggered rules (those whose transient-typed `when`
   premise has a matching fact in the current transaction
   overlay). Collect their head conclusions, partitioned into
   durable assertions and transient assertions.
2. **Retract this round's transients.** They belonged to *t+k*
   and don't carry forward. If they remained visible, the same
   rule would fire again next round on the same fact.
3. Apply the round's durable conclusions to the transaction.
   Carry-forward (implicit persistence) makes them visible at
   *t+k+1*.
4. If this round emitted any transients (effect-emitted), those
   are *t+k+1* facts that can trigger more rules. Continue to
   round *k+1*.
5. If this round emitted no transients, no rule can fire next
   round (every rule requires a transient premise — the
   transient-trigger requirement in [effects.md](./effects.md)).
   Stop. Commit everything as a single durable write.

The termination criterion is **"no transients emitted this
round"**, not "delta is empty". A round that emits only durable
facts cannot trigger further rounds.

### `MAX_ROUNDS` — when it fires

Two pathological shapes hit the bound:

1. **Cycle in the transient-attribute graph.** Effect A produces
   transient `pong` from transient `ping`; effect B produces
   `ping` from `pong`. Each round flips between them; never
   quiesces.
2. **Self-feeding parameterized transient.** A rule whose
   transient head re-triggers itself with a transformed binding
   (e.g. `tick{n}` produces `tick{n+1}`). Each round genuinely
   new, semi-naive dedup can't help.

Both are programmer errors and ideally rejected at rule-compile
time. At runtime `MAX_ROUNDS` (start at 16) caps the work and
fails the commit with a clear error rather than hanging.

What does *not* hit the bound:

- Cascading durable updates triggered by distinct transients —
  bounded by the finite set of transients in flight.
- Tautological rules (`assert!: counter when assert: counter`) —
  semi-naive dedup makes them no-ops; the round emits nothing
  and the loop stops.

### What this means for the builder and reactor

`Commit::perform` runs:

1. Integrate the user's `Changes` into `Transaction`.
2. Move the user-submitted transients (from `TransactionBuilder.transients`)
   into the loop's "current-round transients" pool.
3. Loop:
   a. Find rules triggered by current-round transients.
   b. Fire them; partition heads into durable/transient.
   c. Apply durable heads to transaction; retract current-round
      transients from transaction.
   d. If any transient heads were emitted, they become the next
      round's current-round transients. Otherwise stop.
   e. Increment round counter; error if it exceeds `MAX_ROUNDS`.
4. Commit the transaction.

`retract_transients` as a separate post-loop step disappears in
this model. Retraction happens inline, per round, against the
exact transients that fired *that* round. The builder's
`transients` bucket exists only to seed round 1; after that the
loop tracks transients itself.

## Migration plan

1. **Land the types**: `tonk-schema::transact` module with
   `Application`, `ConceptApplication`, `Statement`, `TransactRequest`.
   No wiring yet.
2. **Extend `TransactionBuilder`**: add the typed `assert(Application)` /
   `retract(Application)` methods and the `transients` bucket. Keep the
   raw `Statement`-typed methods working.
3. **Add the `/transact` route**: parse `TransactRequest`, project to
   builder calls, commit. End-to-end test with a transient concept
   asserting through the new path.
4. **Wire `/evaluate` through the reactor builder**: switch
   `evaluate::run` from raw `branch.transaction()` to
   `tonk_branch.transaction()`, project analyzer statements to typed
   `Application`s with the concept's transient flag.
5. **Implement `evaluate_effects` fixpoint** consuming both buckets
   from the builder (see [effects.md](./effects.md)).
6. **Implement `retract_transients`** as a linear sweep over the
   merged bucket.
7. **End-to-end tests** at reactor level: increment-counter,
   mailbox-with-ack, cascade, silent-drop.

## What this plan deliberately doesn't decide

- Whether `transient` eventually lives on `ConceptDescriptor`
  upstream. Holding that until we've validated the end-to-end mechanism.
- The detailed wire JSON for `ConceptDescriptor` and `Parameters` —
  reuse whatever `/query` does today.
- Concrete analyzer rules for mixing transient and durable concepts
  in one `/evaluate` document. Start permissive; tighten when we hit
  a concrete conflict.
