# Reconciliation via subscription

Retire the account sweep's re-query loop by making the worker a subscriber
to its own branches — the same `Branch::subscribe` path every SSE client
and portal guest already uses. No second sync mechanism: the profile's
view of a space follows the space because something is subscribed to the
space, exactly like everything else in the system.

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
`reconcile_account_spaces`) each open every space's branch, re-run the
same queries, diff against current state, and usually write nothing.
The whole layer is wasm-gated; native (CLI, tests) gets a no-op stub.

## Why subscription is the right mechanism — and what it already provides

The subscription engine (`dialog-repository/.../branch/subscription.rs`)
is pull-driven with a demand gate: same revision + same overlay epoch →
`Ok(None)` at the cost of a compare; a moved tree is intersected with the
query's demand cover; only a touched query re-derives, producing a
`Delta { asserted, retracted }`. The reactor already exploits it on the
hot path — `Pull::perform` (`dialog-reactor/src/pull.rs:77-83`) polls a
branch's subscriptions only when the pulled tree actually moved.

So the engine already computes exactly the deltas the sweep spends
O(spaces) re-deriving every 2 s. Two built-in semantics make it
sufficient without any new primitive:

- **Snapshot on establishment.** A subscriber in `Status::Pending`
  receives a `Frame::Snapshot` of the full current result set before any
  deltas (`dialog-reactor/src/subscription/reference.rs:166`). Catch-up
  and one-time repair are not a separate code path: establishing the
  subscription *is* the repair pass. Today's boot-time sweep becomes
  "subscriptions get established at boot".
- **Diffed writes make dual writes harmless.** A propagated write that
  finds the target already current commits nothing. So a producer may
  write only the source of truth and let propagation follow, or
  proactively write both — in which case the echo from the subscription
  is a no-op. The two differ only in latency (same turn vs. next drain),
  never correctness.

## The gap: the worker never subscribes to itself

Every consumer of a subscription frame today is an **external** client:
an SSE response body (`router/query.rs:149-219`) or a `<tonk-portal>`
guest, for whom the bridge subscribes and pumps envelopes
(`router/bridge.rs` — `.subscribe(query).client(client_id)` plus a
`spawn_local` pump). Deltas computed for `RepositoryName` on a space go
nowhere unless a page happens to be watching.

Closing the gap means the worker does what the bridge already does on
behalf of guests, for itself: open the subscription, consume the
receiver, run a handler per frame. A client of the existing mechanism —
not a registry, not a parallel dispatch system.

For contrast, the two mechanisms that look adjacent but are not this:

- **Commands** (`router/command.rs`) trigger on transients, which don't
  replicate (by design — `plan/effects.md`, "Pull doesn't fire effects").
  Nothing arriving via pull can dispatch one. Commands remain the write
  path; they are not the propagation path.
- **The declarative rule layer** (`rule!:` inductive rules, live via
  commit-time induction; `plan/effects.md` is its design rationale)
  by convention avoids rules
  with persistent-only
  premises, because a *replicated* derivation re-firing on every pull
  breaks convergence under partial replication. Propagation here is a
  **local projection** — each device maintains its own directory labels —
  so re-firing locally on pull is the desired semantics, and the effects
  restriction stays untouched.

## The name flow, end to end

Authoritative record: `RepositoryName` on the space's own `main`.
Directory label: `SpaceName` on profile `main` — it must remain a
materialized fact (not a live join) because the Hub lists spaces this
device has never replicated; for those the seeded label is all there is.

- **Worker self-subscription**, per mounted space: subscribe space `main`
  to `Query<RepositoryName>{ this: space }`. Snapshot and every delta run
  the same handler: write `SpaceName` on profile `main` iff different.
  The "joined before content hydrated" case needs no retry loop —
  hydration is a pull that moves the tree, which fires the subscription.
- **Rename** (`RenameRepository`): the provider writes the space's
  `RepositoryName`; it may keep the proactive `SpaceName` write it does
  today (label updates in the same turn) or drop it (label follows on the
  next drain). Either way the subscription echo diffs to a no-op.
- **Invite seeding**: unchanged — join seeds the directory label from the
  signed `space.name`; the space's own record supersedes it via the
  subscription snapshot once hydrated; a renewal never overwrites.
- **No echo loop**: the propagated write lands on profile `main`; the
  subscription watches space `main`. Different branch. The general review
  rule: a self-subscription handler must write outside its own demand
  cover, or be diffed (then a same-branch write converges in one no-op
  poll).

The other sweeps map the same way:

- `AccountDisplayName → ProfileName + MemberName` fan-out: one
  self-subscription on profile `main` for the account's display name;
  handler projects on actual change instead of every drain.
- `reconcile_founder_membership`: self-subscription on each space's
  `Membership` rows; the snapshot arm runs the migration once per
  establishment, deltas catch stray founder rows later. The handler keeps
  its grant-proof guards and its browser-only identity dependencies —
  capability-gated per handler, not per layer.
- Mount reconcile / local-only adoption: self-subscription on the
  directory rows (`Space` + `Remote`, profile `main`).
- Registration itself: a self-subscription on `Replica` rows maintains
  the set of per-space subscriptions, replacing `real_space_keys`
  re-enumeration.

## Per-target consumer placement

This decides whether the layer actually un-gates, so it is explicit:

- **wasm (service worker)**: a `spawn_local` pump per self-subscription,
  the bridge's exact pattern. Frames arrive when the drain (or an inline
  pull poll) pushes them; the pump wakes, runs the handler, and the
  handler's own commit schedules the target branch's poll so downstream
  subscribers see it on the drain's closing `run_scheduled_polls`.
- **native (CLI, tests)**: request-scoped, no long-lived pump — and none
  needed. `auto_sync` pulls before each command; establishment delivers
  the snapshot; the handler must be **awaited within the command's
  lifetime**, not detached, or the process exits before propagation runs
  and the wasm-only layer is silently reintroduced. Concretely: after the
  pre-command pull, drain the self-subscription receivers to quiescence
  before executing the command.

Handlers follow the dispatch lock discipline (`router/command.rs:285-334`):
capture what's needed under the read lock, drop it, run, then let the
normal poll machinery fan out.

## Consistency and failure

- **Who wins**: unchanged. The space's `RepositoryName` is the source of
  truth; the label always follows it; renewal never overwrites. The
  mechanism moves *when* the copy runs, not which way it points.
- **Failure = re-establish.** The sweep's retry story was "next pass
  re-diffs". A subscriber that fails mid-handler has consumed a delta it
  didn't act on; rather than per-handler watermarks, drop and re-open the
  subscription — the establishment snapshot is the repair. Handlers stay
  idempotent and diffed.
- **Profile-main serialization.** The tear concern behind
  `ensure_account_state_swept`'s mutex (interleaved multi-step commits on
  profile main wedging the worker) doesn't vanish. Single-transaction
  diffed writes serialize on the branch transactor and converge under
  races; any handler doing a multi-step read-modify-write on profile main
  must take the same serialization the sweep takes today.
- **Lifetime honesty**: propagation runs only while a worker (or a CLI
  command) is alive to consume frames. That is the same guarantee the
  sweep gives — it also only runs inside a live worker — with the boot
  catch-up now provided by the snapshot instead of a hand-written pass.

## What stays outside

- `sync_ready`'s non-reconcile duties (`adopt_account_access`,
  `record_activation`, `seed_sealed_inbox`, `describe_own_device`, push) —
  sync lifecycle, not state propagation.
- `stamp_local_spaces` — boot-time non-durable overlay stamps; dies with
  the worker by design.

## Open questions

1. **Rename dual write: keep or drop?** Keeping it gives same-turn Hub
   updates; dropping it makes the space the only write target and accepts
   ~one drain of label lag. Leaning keep, purely for UI latency.
2. **Subscription residency.** One engine per (space × query) held open —
   pinned root + demand cover each. The sweep re-opened every branch per
   pass anyway, so this trades repeated opens for resident state; likely
   a win, worth a number.
3. **Ordering.** Founder repair currently runs before the name projection
   inside one function. Independent self-subscriptions on the same branch
   should either be commutative (preferred) or share one subscription
   whose handler sequences the steps.
4. **Migration safety net.** Keep the old sweep behind a flag through the
   first phase, assert equivalence in tests, then delete — rather than a
   permanent low-frequency backstop.

## Phasing

1. **Pilot: the space-name flow.** Worker self-subscription on
   `RepositoryName` per mounted space (plus the `Replica`-driven
   registrar), replacing `record_space_name` and its sweep site. Tests:
   rename on space main → propagated label after one drain; quiet drain →
   zero handler executions (counter-observable); worker reboot with a
   diverged label → snapshot repairs it; **native run of the same flow**
   proving the un-gating (no wasm stub). Validate with default features —
   `--all-features` silently skips `#[dialog_common::test]` natives.
2. **Account name projection** (`ProfileName` + `MemberName` fan-out),
   deleting the corresponding `converge_account_state` steps.
3. **Founder repair** as a snapshot-triggered self-subscription, identity
   deps behind a per-handler capability bound.
4. **Mounts + local-only adoption**; delete `reconcile_account_spaces`
   and the account-sweep special-casing in `drain_sync` /
   `sync_repository`.
