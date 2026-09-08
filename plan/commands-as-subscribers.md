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

## The executor notion for declarative rules

The V1 transient-trigger requirement guards a real property: a
replicated `effect!:` rule triggered by durable facts would evaluate on
every peer that pulls them. Lifting it needs an **executor** notion —
and Dedalus (the framing `plan/effects.md` is already built on) names
it: the **location specifier**. Every Dedalus fact lives at a site; a
rule whose head is at a different site than its body is a
*communication rule*, and some specific site is responsible for
performing it. Tonk has the geography: branches are the locations, DIDs
are the sites.

Crucially, in Dedalus the location is **not rule metadata — it is a
column of the fact** (`p(X, @L)`), bound and matched by rules like any
other field. So the executor is not an `executor:` annotation on the
rule; it is an audience field on the message, filtered by an ordinary
premise:

```yaml
effect!:
  assert!: invite
  when:
    - assert: invite-request
      where: { for: ?here, ... }
    - assert: db/origin           # injected: THIS replica
      where: { this: ?here }
```

Rules just filter out messages that aren't for them. And the ground
atom this requires **already exists**: every session evaluates with
injected identity facts — `db/session` (profile, operator, active
branches), `db/origin` (repository subject × device profile: *the
replica*, documented in `core.yaml` as usable "to assert/query device
specific information"), and `db/branch` (unique per origin, precisely
because replicas hold same-named branches at different revisions).
Dialog auto-materializes the `dialog.session/*` facts into every
transaction view (`dialog-repository/.../transaction/query.rs:463`);
the concepts are first-class in `dialog-repository/src/schema.rs`.
Nothing to invent, not even an overlay stamp.

This also sharpens the address space beyond "device": the injected
facts form an identity **hierarchy** — profile, operator, replica
(`db/origin`: this device's copy of this repository), branch — and a
rule can resolve "who am I" at whichever level fits, matching `for:`
against it with a direct join. The level chooses the execution
multiplicity: a profile-addressed message matches *every* replica of
that profile (a broadcast to one's own devices — right for device-local
maintenance, where once-per-device is the point), while a
replica-addressed message matches exactly one. Effects with global
outcomes (minting) address a replica — or a role that resolves to one.
Since the identity derives deterministically from device key + repo DID
+ branch name, a replica address is stable across worker restarts and
computable by any peer that knows the target's device profile (from
roster/custody/device facts).

Role addressing then composes *on top* as derivation rather than being
a second mechanism: resolving `for: host` to a concrete replica is a
deterministic monotone rule over roster facts (class 1 below — safe to
evaluate everywhere), whose conclusion is a replica-addressed message.
Prefer role addressing for durable requests all the same: an exact
replica address in replicated state is precise but brittle — a lost or
rotated device leaves messages addressed to a dead replica unconsumed
forever, so replica-exact messages want an expiry or a re-resolution
rule, while role-addressed ones survive re-hosting by re-resolving.

Design consequences:

- **Enforcement by proof, unchanged.** The audience filter is
  placement, not security. The head write requires authority the
  audience can prove (the UCAN chain on the commit); peers validate
  provenance on pull, so a peer that ignores the filter produces
  commits that fail validation. This never depended on where the
  addressing lived — it extends the capability-scoped invocation
  direction (`plan/command-containment.md`): the executor *is* whoever
  can prove the authority the head needs.
- **Message-less triggers compose through messages.** A placement-
  needing rule fired by a plain state change ("when X crosses
  threshold, mint Y") has no message to carry `for:`. The answer is a
  pivot: a monotone rule — safe to run everywhere — derives *a message
  addressed to the audience*; the effectful rule triggers on that
  message. Addressing needs are always expressible by deriving an
  addressed fact, which is exactly Dedalus's communication move.
- **The forgettability lint.** Rule metadata made addressing
  structurally mandatory; a field makes it omittable — an effectful
  rule without an audience premise silently runs on every peer. Restore
  the guard as an install-time check in the same slot as the V1 trigger
  validation: a rule whose head mints a fresh entity (the only
  nondeterminism a declarative head has) must carry an audience-filter
  premise reaching one of the injected identity facts, at a level that
  yields the intended multiplicity (replica or role→replica for
  exactly-once).
- **Which rules need addressing** — the taxonomy that bounds the
  feature:
  1. *Deterministic, monotone derivations* (head a pure function of the
     body, assert-only): **no addressing needed**. Every peer may fire;
     identical conclusions merge idempotently; redundant firing is a
     no-op. The V1 restriction was over-broad for this class — these are
     safe to replicate and evaluate everywhere.
  2. *Nondeterministic or effectful heads* (mint an entity id, sign,
     timestamp, external IO): **addressing required**. The divergence
     risk was never the re-firing — it is two peers deriving *different*
     facts from the same inputs.
  3. *Non-monotone rules* (`retract!:` heads, `unless` over replicated
     state): addressing helps (designate the peer with the
     authoritative view — usually the host), but negation under partial
     replication is hard regardless: absence is indistinguishable from
     not-yet-pulled. Most caution here, independent of placement.

Consumption has a declarative spelling and is separate from the
audience filter (a failed filter is placement, not consumption): the
audience's rule negates its own trigger — an outcome fact plus
`unless: outcome` in the body, or a `retract!:` of the request — the
declarative twin of the imperative side's outcome-gated dedup,
expressible in the existing rule vocabulary.

The identity facts already exist, so what gates lifting the V1
restriction is only the install-time lint and the consumption
discipline; until those land, effects trigger on transient or overlay
facts only.

## Rename dissolves into a fact

With propagation in place, the *space* needs no rename command at all:
`RepositoryName` is an ordinary fact on the space's content branch; a
member with write access asserts it and the label propagates to every
member's profile directory. The copy is class-1 above — deterministic
and directed — so no command is required anywhere in principle.

The profile-level `Rename` survives only as sugar: a same-turn dual
write for UI latency, and a place to validate/normalize the name.
Direct assertion bypasses handler validation — either accept
schema-level constraints as sufficient, or use a guard-rule pattern (a
requested-name fact from which a rule derives the canonical
`RepositoryName`), which is itself a propagator.

**Not "vice versa" as a standing rule.** A standing propagation in
each direction on a cardinality-one register does not converge:
overwrite registers are not lattices, so two live rules ping-pong
whenever the records differ (diff-guards stop the loop, not the
arbitrary winner). The shape that works: **one standing propagation,
space → profile** (the mirror always follows), and a profile-side
rename is an *action* that writes the space's record (plus optionally
the mirror, for same-turn UI). Editing the label from the Hub works by
writing the source of truth, not via a reverse rule. Symmetric standing
sync would require LWW metadata on the name so the merge becomes a
join — possible, unneeded.

Containment consequence: the space vocabulary from PR #911 shrinks —
`RenameRepository` drops out of it entirely.

## Propagators

The converged design is a propagation network in the Radul–Sussman
sense, and the mapping is structural, not decorative:

| Propagator model | Tonk |
|---|---|
| cell | a (branch, query) view |
| propagator | a subscriber: reads input-cell deltas, adds facts to output cells |
| propagator's input registration | the query's demand cover |
| alerted-propagator queue | `pending_polls` / scheduled polls |
| run scheduler to quiescence | drain self-subscribers to quiescence |
| adding present information is a no-op | diffed writes; the proactive dual write's echo has no effect |

One rigor gap keeps it from being propagators all the way down: cells
require merge to be a lattice join (commutative, associative,
idempotent) for confluence. Cardinality-many sets qualify;
**cardinality-one registers do not** — they are overwrite, and their
convergence comes from ordering plus authority, not merge. Hence the
network invariant every new edge must satisfy: a propagator is either
**monotone** (writes lattice-merging facts) or **directed** (one
authoritative source; mirrors always follow, never write back). The
name flow is directed by construction ("space's record wins, renewal
never overwrites"); the executor taxonomy above is the same split seen
from the rules side — class 1 may run everywhere *because* it is
monotone, classes 2–3 need placement *because* they are not.

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

1. **Overlay deltas — resolved by state-layers.** On current dialog
   main the caution was warranted: an overlay write bypasses induction
   (session facts can be read by rule bodies but never trigger a
   rule — `notes/state-layers.md` names this gap explicitly). The
   state-layers branch (`claude/dialog-db-state-layers-7p905m`) is the
   fix: procedural writes join the commit stimulus and rule
   conclusions into the procedural layer are observable to every
   subscription on the branch, both covered by tests in
   `placement.rs`. The ephemeral-command leg therefore builds on that
   branch landing in dialog and tonk advancing its dialog pin, not on
   hand-rolled overlay asserts.
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

## Once-only and no history: the button-click case

An event-triggered command (`Increment` on a + click) keeps both of
its current guarantees in the unified model — neither is threatened by
dropping transience, because durability is a **per-command tier**, not
a consequence of unification:

- **No click history.** Storage tier is a per-command choice, and
  dialog's state-layers branch
  (`claude/dialog-db-state-layers-7p905m`, `notes/state-layers.md` +
  `rust/dialog-repository/src/placement.rs`) makes it a **schema
  property of the attribute** rather than a per-call-site verb:
  `dialog.attribute/layer` declares where an attribute's facts live —
  `semantic` (tree: durable, replicated), `procedural` (session
  overlay: this process only), with `episodic`/`sensory` reserved.
  Writers just `assert`; the commit routes by placement. `Increment`
  declared procedural has zero storage overhead — the branch's own
  test `it_keeps_a_procedural_only_transaction_off_the_tree` proves a
  pure-click batch mints no revision. The counter state persists, the
  clicks never do.
- **Fires once.** Two mechanisms stack. Everything is edge-triggered:
  subscribers consume deltas and induction runs over a commit's
  stimulus / the induce watermark — a fact asserting is one edge, one
  firing; nothing re-evaluates level state per pass. On the
  state-layers branch this holds for ephemeral asserts specifically:
  `it_induces_over_a_procedural_write` shows a procedural write in the
  stimulus firing rules whose semantic heads land in the tree — click
  triggers rule, once, and the click itself never persists. And
  consumption: the handler (or a sweep rule) retracts the procedural
  command when processed, so even snapshot re-establishment within the
  session cannot redeliver. Ten rapid clicks = ten asserts = ten
  stimuli = ten increments; a crash between click and handling loses
  the click, exactly as transients do today. The once-only hazard
  lives only in the durable (semantic) tier — boot deliberately
  replays unconsumed commands — which is why outcome-gating is
  mandatory there and clicks do not belong in it.

## The plan

Build order, each phase the test bed for the next. Phases 1–4 are
imperative Rust on the existing engine; the declarative notation comes
last, once its semantics are proven.

1. **Substrate + name flow** (`plan/reconcile-via-subscription.md`
   phase 1): worker self-subscriptions, pump, establishment snapshot,
   drain-to-quiescence on both targets. The name propagator ships here,
   imperatively — "space name updates profile label" exists from day
   one; only its notation is deferred.
2. **Migrate remaining sweeps** (account-name projection, founder
   repair, mounts); delete `converge_account_state`.
3. **Ephemeral-command leg — adopt state-layers**: once
   `claude/dialog-db-state-layers-7p905m` lands in dialog and the tonk
   pin advances, declare UI command attributes procedural
   (`dialog.attribute/layer`); commands become plain asserts routed by
   the schema, triggering rules and subscribers with no persistence;
   retire the transient capture apparatus. (Dialog-side motivation for
   that branch independently cites the `Provider<C>` registry as
   "re-deriving what induction already computed" — the two efforts
   converge.)
4. **Durable-command leg**: command entities + `for:` audience +
   outcome-gated consumption; pilot on `InviteRequest`-to-host. Note
   the reserved `sensory` layer (replicated, never stored) is the
   eventual natural home for cross-peer messages — a semantic command
   with retract-on-consume approximates it until it's backed.
5. **Declarative lift**: `effect!:` rules take over proven patterns —
   same-branch rules first (with the identity-join filter and the
   install-time lint). Cross-branch edges (the name mirror) stay
   imperative longest: inductive rules are single-branch, so their
   cross-branch story is the addressed-message pivot — though
   state-layers' composite subscriptions (`QueryLayer::subscribe`,
   per-line pins) point at the standing multi-source query that story
   would want. Nothing in phases 1–4 blocks on it.
