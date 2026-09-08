# Commands as subscribers

Follow-on to `plan/reconcile-via-subscription.md`. That doc makes the
worker a subscriber to its own branches for state propagation. This one
explores collapsing command dispatch into the same substrate: effect
providers become subscribers that happen to react to command facts, and
the one-cycle transient lifetime is retired as a bundled default.

## What `transient: true` actually bundled

The current command model (transient concept asserted via `/transact`,
captured pre-commit, matched by the registry, swept from storage) fused
three independent choices into one flag:

1. **Trigger mechanism** — a handler runs when a command fact appears.
2. **Locality** — only the submitting peer ever executes, because
   transients never replicate.
3. **Lifetime** — gone after one commit cycle.

(1) no longer requires transience: a self-subscriber reacts to durable
deltas through the demand-gated engine. The tier-1 cost argument in
`plan/effects.md` — keep triggers transient so a commit with no
transients matches no rules — predates having any demand-gated consumer;
the subscription demand cover provides the same property for durable
facts (an untouched query is a revision compare, a touched one wakes only
the subscribers whose cover intersects).

(3) was never desirable on its own. One-cycle lifetime is why an
unmatched command is silently dropped, why nothing survives a crash, and
why a command arriving **via pull can never execute** — the structural
gap that blocks e.g. "assert an `InviteRequest` addressed to the space's
host; the host executes it when it next pulls, whether or not the
requester is still online."

(2) is the one property worth keeping — per command, not globally.

## The unified model

Every handler is a subscriber: `subscribe(Query<C>)` on the branch(es)
where `C` may be asserted, consuming snapshots and deltas through the
same pump the reconcile plan introduces. A command submission is a plain
commit; the transient capture apparatus in `/transact`
(`transact.rs:303` snapshot-before-commit, `spawn_dispatch`, the
transient sweep) deletes.

The locality axis becomes a per-command choice:

- **Overlay commands** (today's UI-local operations): asserted into the
  session overlay, not the branch. Overlays never replicate and die with
  the worker — the same locality and crash story transients had — but
  overlay writes bump the epoch the subscription engine gates on
  (`subscription.rs:472`, scheduled via `overlay.rs:91`), so the same
  subscriber reacts to them. Lifetime: until the handler retracts the
  overlay fact, not one cycle — a command survives a busy handler instead
  of being swept by the next commit.
  - **To verify before relying on this**: that an overlay assert surfaces
    in the subscriber's `Delta::asserted` (not merely as a re-derivation
    trigger), and that overlay retraction produces the matching
    `retracted` entry.
- **Durable commands** (cross-device / cross-peer requests): plain facts
  on a replicated branch. Crash-surviving, offline-submittable,
  executable on pull by whichever peer is the executor.

## The two invariants transience gave for free

These are the real price of the unification; both need explicit design,
and both only bite durable commands (overlay commands keep the old
semantics by construction).

### Single execution — who runs a replicated command?

Every device subscribed to the branch sees the command delta; two devices
on one account must not both mint the invite. Options:

- **Addressing (preferred)**: every durable command carries an
  `executor` (a DID — the space host, a specific device, the account
  root). Subscribers skip commands not addressed to them. Simple,
  auditable, and matches the host-rooted invite direction where the
  space's host is the natural executor.
- Claim protocol: a cardinality-one `CommandClaim` won via dialog's
  commit-retry-on-version-mismatch. General but a distributed-systems
  tax with no current customer. Defer.

### Consumption — snapshot replay is at-least-once delivery

Snapshot-on-establishment, the property the reconcile plan relies on for
repair, here means every boot **re-delivers unconsumed durable
commands**. That is a feature (crash-surviving retries) exactly when
handlers are gated:

- The handler's commit **retracts the command atomically with writing
  its outcome facts** (one transaction), or
- re-execution is gated on the outcome's existence (command and outcome
  share the command's entity id).

For idempotent handlers (`RenameRepository`) this is hygiene. For
non-idempotent ones (`InviteRequest` mints, `CreateSpaceRequest` creates
a DID) it is load-bearing: outcome-gated dedup is mandatory before any
such command becomes durable. This is `plan/effects.md`'s
mailbox-with-ack pattern promoted from future work to the core of the
command model.

Retention is then a policy choice per command: retract-on-consume (the
outcome is the record) vs. keep-command-plus-outcome as an audit log
with execution gated on outcome presence.

## Containment maps onto subscriptions

The origin-scoped vocabulary split (PR #911, `CommandProviders
{ profile, space }`) becomes structural: *the set of command queries the
worker subscribes to on a branch is that branch's vocabulary*. Space
branches get subscribers for `Load`, `RenameRepository`,
`InviteRequest`, `ExpelMember`; profile main gets the full set. No
dispatch-time vocabulary selection — a command asserted where no
subscriber watches it simply never executes. `may_target_space` stays in
handlers as defense in depth, unchanged.

## Liveness

Today `spawn_dispatch` runs handlers under the fetch event's `waitUntil`
(wasm) or inline (native). The replacement is the rule the reconcile
plan already needs for native: **after a commit or pull, drain
self-subscriber pumps to quiescence within the caller's lifetime** —
under `waitUntil` on wasm, awaited before process exit in the CLI. One
liveness rule, both targets, commands and reconcilers alike. Latency for
the submitting request is unchanged in practice: the transact commit
schedules the poll, the closing drain runs the pump, outcomes land as
facts the UI already subscribes to.

## Declarative effects stay transient-triggered

Do **not** extend durable triggers to `effect!:` rules yet. The V1
transient-trigger requirement guards a real convergence property: a
replicated declarative rule re-firing on every peer that pulls a durable
command would execute everywhere, and rules have no vocabulary for
addressing or consumption. Imperative subscribers escape via
`executor` + outcome-gating; declarative effects can follow only once
they grow the same notions. Until then effects trigger on transient or
overlay facts only.

## What deletes

- Transient capture in `/transact` (`transact.rs:303`), `spawn_dispatch`
  (`transact.rs:197`), the transient sweep from durable storage.
- `dispatch` + `CommandRegistry::match_transients`
  (`router/command.rs:270`, `command.rs:514`) — replaced by the
  subscriber pump; the `Decode` / `Provider<C>` typing survives as the
  subscriber's frame-decoding and capability bounds.
- The "transients arriving via pull never dispatch" gap — dissolved
  rather than fixed.

## Open questions

1. **Overlay deltas** — verify overlay asserts/retracts flow into
   subscriber `Delta`s (see above). If not, that's a small dialog-side
   fix, but it gates the overlay-command leg.
2. **Command entity identity** — durable dedup needs stable command ids
   shared with outcomes. Convention today is loose (`effect:system` as a
   well-known `this`); durable commands likely need caller-minted unique
   entities.
3. **Unaddressed durable commands** — forbid them (lint at subscriber
   registration / schema level), or default `executor` to the submitting
   device? Forbidding is safer; a durable command without an executor is
   a fan-out execution bug waiting to happen.
4. **Backpressure** — a burst of durable commands on establishment (big
   snapshot after long offline) runs handlers in sequence; is per-branch
   serialization enough, or do handlers need a concurrency budget?
5. **Migration order** — which command goes durable first? Candidate:
   `InviteRequest` addressed to the space host (aligns with the named
   invite direction and has a real offline use case), but only after
   outcome-gated dedup exists. Everything else can move to overlay
   commands with near-zero semantic change.

## Sequencing

The reconcile pilot (`plan/reconcile-via-subscription.md`, phase 1)
exercises the self-subscriber substrate — pumps, establishment
semantics, drain-to-quiescence on both targets — with no delivery-
semantics risk. Command migration reuses that substrate:

1. Reconcile pilot lands (name flow).
2. Overlay-command leg: verify overlay deltas, move one UI command from
   transient dispatch to an overlay-triggered subscriber, delete its
   dispatch path.
3. Durable-command leg: command ids + `executor` + outcome-gated
   consumption, pilot on `InviteRequest`-to-host.
4. Retire the transient apparatus once no command depends on it.
