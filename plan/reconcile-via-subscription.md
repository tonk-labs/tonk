# Reconciliation via subscription

Replace the account sweep's re-query loop with delta-driven reconcilers
riding the same subscription engine that feeds SSE. Exploration/assessment;
no implementation yet.

## The sweep layer today

Everything funnels through one path, once per sync drain:

```
drain_sync → ensure_account_state_swept → sync_ready
  → converge_account_state
      → AccountDisplayName → ProfileName projection
      → project_member_names            (per space: MemberName + founder repair)
      → reconcile_account_spaces        (per space: name mirror, mount config,
                                         local-only adoption)
```

- `converge_account_state` — `tonk-worker/src/router/account_state.rs:1367`
  (wasm-only; `:1470` is the native no-op stub).
- `record_space_name` — `router/repository.rs:3391`, called from
  `router/adopt.rs:259` — copies the space's own `RepositoryName` (space
  `main`, authoritative) into the profile directory's `SpaceName` mirror.
- `reconcile_founder_membership` — `router/rotation.rs:334` — migrates
  onboarding-keyed roster rows to the real root, guarded by grant proofs.

A drain runs on a ~2 s self-scheduled loop while a visible tab has live
subscribers (`SYNC_LOOP_MS`, `worker.rs:2344`), plus debounced fetch
triggers. Each sweep pass is **O(spaces)**: `real_space_keys` enumerates
`Replica` rows, then two independent passes (`project_member_names`,
`reconcile_account_spaces`) each open every space's branch and re-run the
same queries, diff against current state, and usually write nothing.
The whole layer is wasm-gated; native (CLI, tests) gets a no-op stub.

## Correcting the framing

"Subscription instead of sweeps" suggests moving from polling to
event-driven. That is not what the subscription machinery is. **Nothing in
tonk is push-driven**: there is no commit-notification stream from remotes;
the clock is the same drain loop, and subscriptions are re-polled from it.
What a subscription adds is the **demand gate**
(`dialog-repository/.../branch/subscription.rs:457`):

- same revision + same overlay epoch → `Ok(None)`, zero work;
- tree diff intersected with the query's demand cover → touched or not;
- touched → incremental re-derivation, producing a `Delta { asserted,
  retracted }`.

And the reactor already exploits it on the hot path: `Pull::perform`
(`dialog-reactor/src/pull.rs:77-83`) polls a branch's subscriptions **only
when the pulled tree actually moved**. An idle drain does zero subscription
work.

So the honest claim is not "event-driven instead of polling" but:
**the engine already computes exactly the deltas the sweep spends O(spaces)
re-deriving — and then hands them only to SSE clients.** The sweep exists
because there is no way to register in-process code as a delta consumer.
That is the gap to close.

## What exists, what's missing

| Primitive | Reacts to | Runs user code? | Cross-branch write? |
|---|---|---|---|
| `Subscription::poll` (dialog) | tree diff vs demand cover | no — produces `Delta` | read-only |
| `run_scheduled_polls` fan-out | scheduled/pulled branches | no — SSE frames only | no |
| `CommandRegistry` dispatch (`router/command.rs:270`) | **transient** asserted in a local `/transact` | yes (`Provider<C>::execute`) | yes — the only mechanism |
| dialog induction (`dialog.rule/on`) | local commit's touched attributes | declarative head only | same branch |
| `PendingSubscription` adoption | branch materialization | no | no |

Missing: a consumer of **durable deltas** (including pulled ones) that runs
Rust and may write other branches. Commands can't be it: transients don't
replicate (by design — `plan/effects.md`, "Pull doesn't fire effects"), so
nothing pulled ever dispatches. Effects V1 can't be it either: it
deliberately rejects rules with persistent-only premises, because a
replicated derivation re-firing on every pull breaks convergence under
partial replication.

That rejection is correct for **replicated** state transitions — and
irrelevant here. The sweep layer is a different species: **local
projections and repairs**. Its outputs are either device-scoped (mount
config, overlay stamps) or idempotent copies whose source is authoritative
(name mirror, roster migration). Every peer maintaining its own projection
by re-firing locally on pull is exactly the desired semantics, not a
convergence bug. Effects and reconcilers are complementary layers, and the
transient-trigger restriction on effects stays untouched.

## Design sketch: a reconciler registry

Mirror the command registry's shape, but keyed on durable queries instead
of transient concepts:

```rust
trait Reconciler<Env> {
    /// The branch scope + query whose deltas this reconciler consumes.
    fn subscription(&self) -> (BranchScope, Query);
    /// Establishment: runs once over the full current result set
    /// (the level-trigger — covers repair of pre-existing state).
    async fn establish(&self, env: &Env, snapshot: &[Row]) -> Result<()>;
    /// Steady state: runs on each non-empty delta.
    async fn react(&self, env: &Env, delta: &Delta) -> Result<()>;
}
```

Wiring:

- The registry holds one dialog `Subscription` engine per registered
  (branch × query), registered as an internal subscriber on the
  `BranchState` — polled by the same `schedule_poll` /
  `run_scheduled_polls` / pull-inline machinery that already exists. No new
  clock.
- Handler execution follows `dispatch`'s lock discipline
  (`router/command.rs:285-334`): collect deltas under the read lock, build
  `'static` futures, drop the lock, `join_all`, then one
  `run_scheduled_polls` so the reconcilers' own writes fan out in the same
  turn.
- **Establishment = boot sweep.** Subscription state is in-memory, so
  every worker boot re-establishes and `establish` runs over the snapshot.
  Today's boot-time full pass falls out of the mechanism instead of being
  a separate code path.
- **Failure = re-establish.** The sweep's retry story is "next pass
  re-diffs". A delta consumer that fails has consumed a delta it didn't
  act on; rather than inventing per-handler watermarks, drop the failed
  reconciler's engine and re-create it — the establishment snapshot is the
  repair pass. Handlers stay idempotent and diff-before-write (they
  already are).
- Registration is per-space for space-scoped reconcilers, driven by a
  profile-scoped reconciler over `Replica` rows — the registrar is itself
  a reconciler, replacing `real_space_keys` re-enumeration.

An alternative hook exists — `Branch::induce`
(`dialog-repository/.../transaction.rs:280`, the documented "post-pull
instant", never called from tonk) — but it runs declarative rules on one
branch. The reconcilers need imperative cross-branch writes, so the
registry is the right layer; `induce` remains available for declarative
same-branch level triggers later.

## The name-sync case, expressed on it

1. **Space name mirror** (replaces `record_space_name` + its sweep site):
   per space, subscribe space `main` to `Query<RepositoryName>{ this:
   space }`. On delta (and establishment), write `SpaceName` on profile
   `main` iff different. The "content not yet hydrated" case needs no
   retry loop: hydration is a pull that moves the tree, which triggers the
   subscription.
2. **Account name projection** (replaces `converge_account_state` steps):
   subscribe profile `main` to `Query<AccountDisplayName>{ this: root }`.
   On delta, update `ProfileName`, then fan `MemberName` out to each
   space's roster and `mark_dirty` — same writes, but only when the name
   actually changed instead of diffed every 2 s.
3. **Founder repair**: level-triggered — `establish` over
   `Query<Membership>{ role: FOUNDER }` per space runs the migration once
   per boot/mount; deltas re-run it if a stray founder row lands later.
4. **Mount reconcile / local-only adoption**: reconciler on the directory
   (`Space` + `SpaceName` + `Remote` rows, profile `main`).

### Echo analysis

- Mirror write (1) lands on **profile** main; the trigger subscription
  watches **space** main. Different branch — no echo possible.
- `MemberName` writes (2) land on space main, which (1) also watches — but
  the demand cover is fact-range-scoped: `MemberName` assertions don't
  intersect a `RepositoryName` query's cover, so no false re-derivation.
- Same-attribute self-triggering (a reconciler writing what it watches) is
  the only real loop shape; diff-before-write converges it in one extra
  no-op poll. Keep it as a review rule: a reconciler must either write
  outside its own demand cover or be a diffed fixpoint.

### Who wins when both records changed

Unchanged, and worth stating: the mechanism moves *when* the copy runs,
not *which way it points*. The space's `RepositoryName` stays the source
of truth; the profile `SpaceName` mirror always follows it; an invite
renewal never overwrites a hydrated name. Conflict semantics live in the
handlers, exactly as today.

## What it buys

- **Per-drain cost**: O(spaces) branch opens + queries → O(watched
  branches) revision compares (the `Ok(None)` fast path), with real work
  only on actual change. The 2 s loop stops being a 2 s full sweep.
- **Un-gates the layer.** Registration and handlers are plain reconciler
  code with `Provider`-style env bounds — they compile everywhere. On
  native, the CLI already pulls before each command (`auto_sync`); the
  pull-inline poll runs the same reconcilers, so CLI and tests get
  reconciliation for free instead of a stub. Only the founder repair's
  browser-only identity dependencies (`identity::root_did`,
  `prove_path`) stay gated — per handler via a capability bound, not per
  layer. This is the `target-agnostic-providers` argument applied to
  reconciliation.
- **One mechanism, not N call sites.** The five ad-hoc
  `converge_account_state` invocation sites and the account-sweep special
  case in `sync_repository` collapse into "polls run, reconcilers react".

## What stays outside it

- `sync_ready`'s non-reconcile duties (`adopt_account_access`,
  `record_activation`, `seed_sealed_inbox`, `describe_own_device`, push) —
  sync lifecycle, not state reaction.
- `stamp_local_spaces` — writes non-durable overlays that die with the
  worker; establishment-time work, arguably expressible as `establish`
  with no `react`, but fine as a boot task.
- The serialization concern behind `ensure_account_state_swept`'s mutex
  (interleaved profile-main commits tearing an artifact) doesn't vanish:
  reconcilers writing profile main must go through the same transactor
  path; the registry should serialize handlers per target branch.

## Open questions

1. **Ordering.** Today founder repair runs before the name projection
   inside one function. Independent reconcilers on the same branch need
   either declared ordering or proven commutativity. Probably: registry
   preserves registration order per branch, and handlers stay commutative
   where possible.
2. **Engine residency.** One subscription engine per (space × query) held
   open — pinned root + demand cover per space. Modest memory, but O(spaces)
   resident engines vs today's transient queries. Likely a win (the sweep
   re-opens every branch anyway), worth measuring.
3. **Delta vs snapshot in handlers.** Some handlers (name mirror) want the
   new value only; others (founder repair) want the full row set. The
   `establish`/`react` split covers it, but handler signatures should
   receive typed concept rows, not raw frames — reuse the `Decode`
   machinery from `dialog-reactor/src/command.rs:95`.
4. **Safety net.** Is establishment-on-boot enough, or keep a
   low-frequency full sweep as belt-and-braces during migration? Proposal:
   keep the old sweep behind a flag through phase 2, assert equivalence in
   tests, then delete.

## Phasing

1. **Registry + pilot: the space-name mirror.** Land `Reconciler`, the
   internal-subscriber wiring, establishment semantics; port
   `record_space_name`. Tests: rename on space main → one drain → mirror
   updated; quiet drain → zero reconciler executions (observable via a
   counter); worker reboot → establishment repairs a mirror diverged while
   down. Native test proves the un-gating (no wasm stub).
2. **Account name projection** (`ProfileName` + `MemberName` fan-out),
   deleting the corresponding `converge_account_state` steps.
3. **Founder repair** as a level-triggered reconciler, identity deps
   behind a per-handler capability bound.
4. **Mount reconcile + local-only adoption**; delete
   `reconcile_account_spaces` and the sweep's special-casing in
   `drain_sync`/`sync_repository`.
