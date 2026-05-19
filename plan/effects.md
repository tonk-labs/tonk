# Effects — inductive rules for tonk

## Context

Phase 1 landed `InductiveRule` upstream in `dialog-query` (commit
`83549c03` on branch `feat/inductive-rule`). The dialog-side type
is a full inductive rule: when the body matches, the head is
asserted into the branch. Compilation runs the shared
`Compile::compile` pipeline (planner + unbound-variable check).
The serialized descriptor uses the `assert!:` wire-format key for
the head.

What this plan covers is **everything tonk needs on top**:

1. Storing inductive-rule definitions inside a branch as
   `dialog.effect/*` facts so they replicate.
2. Evaluating them at transaction time via a fixpoint loop in the
   reactor.
3. Surfacing them as "abilities" the UI can render and trigger.

### Conceptual framing: Dedalus, transients, and persistence

The theoretical foundation is Dedalus. In Dedalus, every relation
is empty at every timestep unless some rule produces it.
Persistence is a special case of inductive rules: a fact persists
because there's an explicit rule `p(X)@next :- p(X)` saying "if
p(X) now, then p(X) next." Without that rule, `p` is empty next
round.

Dialog's storage layer is already this rule, materialized inside
the engine. Every EAV triple carries forward unless a retract for
the same entity+attribute pair is emitted. Stated as a tonk-yaml
rule, the implicit persistence rule is something like:

```yaml
rule!: &persist
  assert!: assert
  when:
    - assert: assert
      where: { this: ?this, the: ?the, of: ?of, is: ?is }
  unless:
    - assert: retract
      where: { this: ?this, the: ?the, of: ?of, is: ?is }
```

Dialog doesn't actually evaluate this rule: the storage layer
does its job directly. But it's a faithful description of the
semantics, and it gives us vocabulary for talking about
exceptions to it.

The model that follows is built on a few core decisions, locked
through several rounds of design:

#### Concept persistence is a per-concept declaration

Every concept declaration includes (implicitly or explicitly) a
**`transient`** flag:

- **`transient: false` (default)**: facts of this concept get the
  implicit persistence rule applied. They carry forward across
  timesteps until retracted.
- **`transient: true`**: facts of this concept have no implicit
  persistence rule. They exist only at the timestep they're
  submitted in, and are stripped from the persistable delta
  before the branch state is written.

Transient concepts are how messages, commands, events, and
abilities are expressed. Their one-timestep lifetime is what
makes them safe under partial replication: they never enter
storage, never replicate, can only fire effects on the peer
where they're submitted.

#### Carry-forward is governed by retraction only

The implicit persistence rule applied to persistent concepts is
exactly:

> The value at entity+attribute pair `(of, the)` at the current
> timestep carries forward to the next timestep, unless a
> retraction for that pair is emitted at the current timestep.

This is uniform across cardinalities. What differs is what
happens when new values *also* land on the same pair:

- **Cardinality-one**: storage's "at most one value per
  entity+attribute" rule kicks in. The new value replaces the
  old at write time. Authors updating a cardinality-one value
  just assert the new value; they don't need to retract the old
  one explicitly.
- **Cardinality-many**: multiple values can coexist. The implicit
  carry-forward keeps existing values; new asserts add more.
  Authors who want to remove a specific value write an explicit
  retract.

This matches the natural intent of each cardinality declaration:
cardinality-one means replacement, cardinality-many means
accumulation.

#### Rule heads have two polarities

Inductive rules have either `assert!:` or `retract!:` at the
head:

```yaml
rule!:
  assert!: counter
  when: …
  # Body bindings fill the head's fields; head produces an
  # assertion of `counter`.

rule!:
  retract!: message
  when: …
  # Body bindings identify the cells to retract; head produces
  # retracts for those cells.
```

Each rule has one polarity. Multiple rules can share the same
head concept and compose by disjunction, the same way deductive
rules compose. Adding a new event to the system means adding new
rules; existing rules don't need to change. This is the
extensibility property: an `assert!: account` rule for a new
event doesn't require the existing deposit/withdrawal rules to
know about it.

#### Conflict resolution: change wins over no-change

When two rules emit on the same cell in the same round:

- **Retract wins over assert.** A retract is an explicit removal;
  an assert that would have produced the same value is
  redundant.
- **Update wins over no-change.** A retract from a rule plus the
  implicit carry-forward yields the retract.

In practice conflicts are rare because rule bodies differ. The
policy is a defensive default for the cases that do arise.

#### Effects, mailboxes, abilities all reduce to the same thing

An **effect** is an inductive rule whose body reads at least one
transient concept's facts. The transient is the trigger: the
rule fires when the transient is submitted, produces persistent
state via its head, and the transient is gone next timestep.

What today's design vocabulary calls:

- An **ability** is a transient concept the UI submits to invoke
  a behavior. Same mechanism.
- A **mailbox message** sent to an inbox is a persistent concept
  (it has to replicate). Consumption is via an explicit `retract!:
  message` rule that fires when an ack (transient or persistent)
  arrives. Same mechanism, with the ack as the trigger.
- **Authorization-as-premise** falls out for free: persistent
  authorization assertions gate rules by appearing as positive
  premises. No special machinery.
- **Expiry** requires a clock concept or built-in `now` formula
  and a `retract!:` rule guarded by the expiry condition. The
  retract rule pattern composes; new expiry policies add new
  rules.

The same mechanism (transient concepts + assert/retract rule
polarities) covers all of these.

#### Effect trigger requirement

For an inductive rule to be installed as an effect, its body
must include **at least one positive `when` premise reading a
transient concept's facts**. This is enforced at effect-
registration time.

The reason is convergence under partial replication. A rule with
only persistent premises would need to re-fire on every pull to
stay in sync with remote facts. Requiring a transient premise
means the rule only fires when *this peer* submits the transient
locally; whatever the rule produces is persistent and replicates
to other peers as plain state.

> [!note]
>
> *Naming.* The word "effects" is overloaded with `dialog_effects`
> (the framework for system IO surface: archive, authority,
> memory handlers). Those are operational effects produced by
> handlers; the effects in this document are declarative
> state-transition specs. Same word, different layer. Internal
> Rust type is `InductiveRule` (upstream); the tonk-yaml surface
> keyword is `effect!:`.

## What an effect looks like

In dialog-yaml, the surface authors write:

```yaml
# A command concept marked transient. Submissions of this
# concept exist for one commit cycle, then they're gone.
concept!: &increment
  description: Command to increment a counter
  transient: true
  with:
    subject:
      the: tonk.xyz.command/subject
      as: entity

# An effect: when an `increment` command lands and there's a
# counter for the target, derive a new counter row with count+1.
# Because `increment` is transient, this rule fires once per
# submission and can't re-fire later.
effect!:
  description: Increment a counter on increment command
  assert!: counter
  when:
    - assert: counter
      where:
        this: ?this
        count: ?last-count
    - assert: increment
      where:
        subject: ?this
    - assert: +
      of: ?last-count
      with: 1
      is: ?count
```

Reading the rule: `assert!:` produces persistent head facts; the
`when` body must match (positive premises in `assert:` form,
negative premises in `unless:`); the head's variable bindings
come from variable names that match the head's field names. The
critical premise is the `increment` one: because the `increment`
concept is declared `transient: true`, this premise reads a
transient fact, which satisfies the effect-trigger requirement.

The counter's value goes from `?last-count` to `?count` via the
`+` formula. Because `counter.count` is cardinality-one, storage
replaces the old value with the new one automatically. No
explicit retract needed.

### V1 transient mechanism

A concept marked `transient: true` has no implicit persistence
rule. When a fact of a transient concept is submitted in a
commit:

1. The transactor admits it for the duration of this commit's
   effect evaluation.
2. Effects can read it via positive `when` premises.
3. Before the persistable delta is written, the transactor
   strips all facts of transient concepts.

The mechanism is just "no implicit carry-forward for facts of
transient concepts," directly implementing the Dedalus rule "if
no rule produces `p(X)@next`, then `p(X)` doesn't exist at the
next timestep."

Four consequences:

- **Transients don't replicate.** Because they never enter
  storage, they never reach the upstream tree, never get pushed,
  never get pulled. This is the property that makes V1 effects
  safe under partial replication: only the peer that originated
  the submission ever sees it.
- **A submission that matches no effect is silently dropped.**
  An unanswered command does not sit in storage. The semantic is
  "submitting a transient is sending a message; the message
  lifetime is one commit cycle."
- **No leftover triggers from a crash.** A crash mid-cycle drops
  the whole transaction; nothing partially-persisted to recover.
- **`effect:system` is convention, not mechanism.** Ability
  concepts (transient commands the UI submits) conventionally
  use `effect:system` as the `this` field for ergonomic reasons
  (one well-known place to send commands), but the transactor
  doesn't special-case the URI. Transience comes from the
  concept's declaration, not from a sentinel entity.

### V1 effect-trigger requirement

An `effect!:` rule (assert or retract polarity) must include **at
least one positive `when` premise reading a transient concept**.
This is checked at effect-registration time (tonk-side, not
upstream — `InductiveRule::new` itself accepts any well-formed
inductive rule).

Rules that don't satisfy this restriction are valid inductive
rules upstream, but tonk's reactor refuses to install them as
effects. The reason is convergence under partial replication: a
rule that reads only persistent concepts would need to re-fire
on every pull to stay in sync with remote facts. With partial
replication, the peer can't even see the changes that would need
to trigger the rule.

Requiring a transient premise means the rule only fires when
*this peer* submits the transient locally. Whatever the rule
produces is persistent and replicates to other peers as plain
state, so the rule doesn't need to re-fire elsewhere. Different
peers converge by replicating each other's outputs, not by each
re-running the rules.

The error message points at the missing trigger and suggests
declaring one of the body's premises against a transient
concept. Authors who want a derived view of state should use a
deductive rule (computed on query); authors who want a state
transition triggered by a local command or event use an effect.

## Architecture

### Layer 1 — `InductiveRule` (upstream, done)

Landed in `dialog-query` on `feat/inductive-rule`:

```rust
pub struct InductiveRule {
    /// The head: a ConceptDescriptor (just like DeductiveRule).
    conclusion: ConceptDescriptor,
    /// Compiled body, planned and validated.
    join: Conjunction,
}
```

Compilation runs the shared `Compile::compile` pipeline:

- Planner ordering and unsatisfiable-premise detection.
- Unbound-variable detection (head and negation premises).

`InductiveRuleDescriptor` is the serializable form with
`assert!:`, `when`, `unless` fields. Both kinds (deductive and
inductive) wrap into the `Rule` enum so analysis errors report
uniformly.

The dialog-side type is general — no trigger requirement, no
mailbox semantics. The V1 effect restriction lives entirely on
the tonk side; upstream stays semantically clean ("an inductive
rule asserts its head when the body matches; runtime decides
where and when to fire it").

### Layer 2 — effect storage in the branch

Effects are themselves *concepts* — `dialog.effect/*` triples —
so they replicate, version, and query like everything else:

```
dialog.effect/assert    : entity   (head concept)
dialog.effect/premise   : entity   (positive when premise)
dialog.effect/unless    : entity   (negative premise)
dialog.effect/description : text
```

The premise/parameter substructure (`dialog.effect/premise →
dialog.premise/predicate`, `dialog.application/parameter`,
`dialog.parameter/name`, `dialog.parameter/binding`) mirrors how
attribute and concept declarations already serialize themselves
into facts. Most of it comes for free from the existing
attribute/concept fact representation; the new bits are
`dialog.effect/{assert,premise,unless,description}`.

There is **no** `dialog.effect/ability` field. Abilities are
*inferred*: any positive premise reading a transient concept
exposes that concept as an ability. The body's premise structure
tells the UI what the ability looks like (its fields, what it
targets).

### Layer 3 — effect loader + V1 validation

A query-builder returns inductive rules currently asserted in a
branch, validated for the V1 effect restriction:

```rust
fn load_effects(branch) -> Vec<InductiveRule>
```

Steps for each `dialog.effect/assert` triple:

1. Reconstruct `InductiveRuleDescriptor` from the
   `description`/`premise`/`unless` fields.
2. Compile via `InductiveRuleDescriptor::compile` (upstream
   analysis: planner + unbound-variable check).
3. **Validate the V1 effect restriction**: at least one positive
   premise must read a transient concept (one whose declaration
   has `transient: true`). Reject (with a diagnostic fact) if
   not.

> [!note]
>
> Failed-compilation handling: we should not lose an entire
> branch because one effect has a broken `where` or doesn't
> satisfy the V1 trigger restriction. Log the error, skip the
> effect, surface it through a `dialog.effect/error` fact (or
> equivalent) so the LSP can show it. This matches the editor's
> existing diagnostic surface for `attribute!` / `concept!`
> errors.

### Layer 4 — semi-naive fixpoint evaluator (in the reactor)

The reactor's commit pipeline currently looks roughly like:

```
apply_local_txn(branch, txn);
notify_subscribers(branch);
```

The new step sits between them:

```
apply_local_txn(branch, txn);
evaluate_effects(branch, txn);   // <-- new
notify_subscribers(branch);
```

The naive shape would be "load every effect on every commit and
re-evaluate." That's correct but does work proportional to the
total effect set even when nothing relevant changed. Instead,
evaluate **demand-driven**: only run effects whose body could
possibly have been affected by what changed.

**The reverse index is a standing query, not a cached structure.**
Effects are themselves stored as facts (`dialog.effect/assert`,
`dialog.effect/premise`, `dialog.effect/unless` — see Layer 2),
so "effects whose body mentions concept `X`" is just a query
over `dialog.effect/premise` rows whose predicate is `X`. There
is no separate in-memory index to maintain, no invalidation on
effect-set change, no cold-start refresh — the lookup is always
live with the branch state.

Conceptually:

```
ConceptId -> Vec<EffectId>   // computed per round by query
```

Both `when` and `unless` premises participate. A retraction of a
`disable` fact can newly enable an effect, so negative premises
need invalidation just like positive ones. Built-in formula
premises (`+`, `>`, etc.) don't index a concept; they're
evaluated as part of whichever rule contains them.

**The loop.** Each round computes the set of attributes whose
facts changed, queries for the candidate effects, fires what
matches. Facts of transient concepts are held in a working set
during the loop and never persisted; only persistent assertions
and retractions land in the durable delta.

```rust
fn evaluate_effects(branch, initial_txn) {
    const MAX_DEPTH: u32 = 16;
    let mut dirty: HashSet<AttributeId> = attributes_touched(&initial_txn);
    // Transient working set: facts of transient concepts. Carried
    // through the loop but never written to durable storage.
    let mut transients = initial_txn.filter(|f| concept_of(f).is_transient());
    let mut persistable = Transaction::new();
    let mut depth = 0;

    loop {
        if dirty.is_empty() || depth >= MAX_DEPTH { break; }

        // Standing query: which effects mention any of these attributes?
        let candidates = query_effects_by_premise_attributes(branch, &dirty);
        let mut delta = Transaction::new();

        for effect_id in candidates {
            let effect = branch.load_effect(effect_id);
            // Evaluate the body against branch state + working transients.
            let bindings = run_query(branch, &transients, &effect.join);
            for tuple in bindings {
                match effect.polarity() {
                    Polarity::Assert => delta.merge_assert(effect, &tuple),
                    Polarity::Retract => delta.merge_retract(effect, &tuple),
                }
            }
        }

        if delta.is_empty() { break; }
        if depth + 1 >= MAX_DEPTH {
            log_runaway_effects(&candidates, depth, &delta);
        }

        // Partition the round's delta into transient (working set)
        // and persistent (durable). Apply persistent immediately so
        // the next round's query sees the updated branch.
        let (next_transients, durable) = delta.partition(|f| concept_of(f).is_transient());
        persistable.merge(durable.clone());
        branch.apply(&durable);
        transients = next_transients;
        dirty = attributes_touched(&delta);
        depth += 1;
    }

    // Transient facts from initial_txn or produced during the loop
    // are dropped here — they only live for the duration of evaluation.
    persistable
}
```

A few notes on the loop's structure:

- The reverse index is keyed by **attribute URI**, not concept
  entity. Asking "which effects could be affected by a change
  to attribute `X`" is a single one-hop query against
  `dialog.effect/premise`. Concept-level invalidation falls out
  naturally because a concept's attributes are what change in
  storage.
- Both `assert!:` and `retract!:` rules participate. The
  evaluator dispatches on the rule's polarity when constructing
  the delta.
- Conflicts within a round (assert and retract on the same cell)
  resolve by retract-wins.
- There is **no cold-start round.** V1 effects require transient
  triggers, and transients don't persist, so there's no
  backlog of pending triggers at branch open. The loop runs
  only in response to a commit.

**Why this handles chain reactions correctly.** A V1-valid
cascade uses `effect:system` for intermediate steps. Take an
example where an `increment` command bumps a counter, and
optionally fires a `notify` command if the new count crosses a
threshold:

```
commit: assert(increment{this: effect:system, subject: counter:1})
round 1: dirty = {increment, counter}
         → fires counter-update rule
         → delta: assert(counter{count+1}),
                  assert(notify{this: effect:system, subject: counter:1})
                    [conditionally, when count crosses threshold]
round 2: dirty = {counter, notify}
         → fires notify rule (reads notify from effect:system)
         → delta: assert(notification{...})
round 3: dirty = {notification}, no effect indexes notification,
                  candidates = ∅
         → loop ends
```

Subscribers see one notification with the settled state. Note
that `notify` (the intermediate command) lives on
`effect:system`, so it's commit-scoped and never persists — only
`counter` and `notification` end up in durable storage.

Per the design Qs already answered:

- **Fixpoint with depth limit.** One commit produces one external
  notification; cascading rule firings are internalized as N rounds
  capped at MAX_DEPTH (start with 16).
- **Always retract trigger** (when one exists). Trigger fact
  retraction is part of the same delta as the head assertion, so
  it's atomic per round.
- **New step between commit and notify.** Effects feel like a
  first-class branch-level computation, sitting alongside the
  existing reactor steps.
- **Demand-driven, not bulk evaluation.** Reverse index over
  premise predicates means we only consider effects whose body
  could possibly have been affected. Subscription-set filtering
  becomes unnecessary; correctness comes from the index, not from
  knowing what anyone's looking at.

### Layer 5 — ability discovery for subscriptions

Each subscription on a concept `C` should *also* tell the client
what abilities are available against rows of `C`. Discovery is
orthogonal to the evaluator — it walks the effect set looking for
command-shaped trigger premises:

```
for effect in load_effects(branch):
  for premise in effect.when:
    if premise reads a transient concept X with a field binding
       to a variable that the head concept also binds:
        if the head concept is C (or a related concept):
            yield Ability { concept: X, on: C }
```

Note this is a **UI-affordance** question, not a correctness
question. The evaluator doesn't need it; the index drives
firing. Discovery is purely about telling the client "here are
buttons you could render." So the simple filter is fine here — if
an ability is invoked by indirect cascade (no direct trigger
premise targeting `C`), there isn't really a button to render for
it anyway; the cascade fires from a different button somewhere
else.

On the wire, the SSE frame today is `data: <Vec<Conclusion>>\n\n`.
We add a sibling event:

```
data: { "abilities": [{"concept":"increment","on":"todo"}] }\n\n
```

…or use a tagged enum. Concrete wire shape is a follow-on detail.

## Chain reactions, resolved by the reverse index

An earlier draft worried about "transitively relevant" effects:
how does a cascade where effect B reads what effect A produces
work without re-evaluating everything?

The reverse-index loop in Layer 4 handles it. Each round
re-computes the dirty set from the previous round's delta and
queries which effects could be triggered by those concepts.
Cascades unfold one fixpoint iteration at a time. No subscription
filtering — correctness comes from the index, not from knowing
what anyone's looking at.

For V1, the intermediate steps of a cascade typically live on
`effect:system` (commands triggered by other effects), so they're
ephemeral. The durable outputs are just the head facts of each
firing.

A hypothetical V2/V3 example where cascades go through durable
intermediates (an effect asserts `counter-snapshot`, which
another effect reads to assert `alert`) is *not* V1-valid because
neither of those effects has an `effect:system` trigger. With V2
mailbox semantics or V3 full re-evaluation, the same shape would
work; in V1 the same logic has to be expressed by having the
first effect's head include an `effect:system` command that
triggers the second effect.

For reference, here's the V2/V3-shaped version we're *not*
implementing yet:

```yaml
# Neither of these is V1-valid (no effect:system premise).
# Both would be accepted once V2/V3 ships.

effect!:
  assert!: counter-snapshot
  when:
    - assert: counter
      where: { this: ?c, count: ?n }

effect!:
  assert!: alert
  when:
    - assert: counter-snapshot
      where: { this: ?c, count: ?n }
    - assert: >
      of: ?n
      with: 100
      is: true
```

Under V2/V3 semantics the cascade would be: when `counter`
changes, the index says "Effect 1 reads `counter`," round 1
fires it and asserts `counter-snapshot`; round 2's dirty set
contains `counter-snapshot`, Effect 2 fires and asserts `alert`.
The reverse-index mechanism is the same; the only difference is
how the runtime decides which rules to consider on a given
trigger (commit, pull-mailbox-scan, or full re-evaluation).

## Phases

### Phase 1 — `InductiveRule` + descriptor (done)

Committed upstream on `feat/inductive-rule` (dialog-db
`83549c03`). Adds `InductiveRule`, `InductiveRuleDescriptor`,
`Rule` enum, `Compile` trait. Tonk re-exports via
`tonk-schema/src/rule.rs`.

### Phase 2 — effect storage (done)

- Schema constants for
  `dialog.effect/{source,conclusion,polarity,premise}` in
  `tonk-schema`. The head-concept-index attribute is named
  `conclusion` (matching upstream `InductiveRule::conclusion()`)
  so the same name works for both assert- and retract-polarity
  effects.
- Reverse index (`dialog.effect/premise`) keyed by attribute URI,
  not concept entity. Handles lens-sharing across concepts: a
  change to a shared attribute correctly invalidates effects that
  read it through any concept lens.
- `Effect::new(rule, polarity)` and convenience
  `Effect::asserting(rule)` / `Effect::retracting(rule)`.
- `Effect::by_entity(branch).resolve(...)` loads source +
  polarity and rehydrates the `Effect`.
- `Effect::validate(branch, env)` enforces the V1 trigger
  requirement at install time: at least one positive `when`
  premise must read a transient concept (lookup via
  `TransientConcept::is_transient` against the branch). The
  check moved from construction to install because it needs
  branch state for transience markers.
- `TransientConcept` wrapper in `tonk-schema::concept`. Same
  storage as `AnonymousConcept` plus a
  `(?this, dialog.concept/transient, db:transient)` marker. The
  marker is what the reactor reads to decide which facts to
  retract.
- `effects_by_premise(attribute)` reverse-index query.

### Phase 3 — semi-naive fixpoint evaluator

**Architecture:** `Commit::perform` now routes through a dialog
`Transaction` instead of a raw `Changes` stream. Sequence:

1. Load user's `Changes` into a fresh `Transaction` via
   `branch.transaction().integrate(user_changes)`.
2. `evaluate_effects(branch, txn, env)` — fixpoint loop.
   Reads `txn.query()` for the overlay view (branch state +
   pending writes). Each round queries the reverse index for
   candidate effects, fires what matches, integrates head facts
   back into `txn`. Loop until empty round or MAX_DEPTH=16.
3. `retract_transients(branch, txn, env)` — for each fact in
   the transaction whose attribute belongs to a transient
   concept (looked up via the `dialog.concept/transient`
   marker), retract it in-place. The assert+retract pair in
   the same transaction collapses at commit, so transient
   facts never reach durable storage. No commit-time stripping
   logic needed.
4. `txn.commit().perform(env)` — durably write the persistent
   residue.

**Hook scaffold landed** (`reactor::effects::evaluate_effects` /
`retract_transients`); both pass the transaction through
unchanged today.

**Still to land:**

- Single-round evaluator: query reverse index, load candidate
  effects via `Effect::by_entity`, run rule body against
  `txn.query()`, instantiate head from bindings, integrate.
  Test: increment-counter (assert polarity, transient trigger).
- Fixpoint loop with MAX_DEPTH=16 and runaway warning.
- Retract-polarity dispatch (the `retract!:` rule head case).
  Heads of retract rules name a concept and identify the cells
  to retract via the bindings.
- Conflict resolution at integration time: assert+retract on
  the same cell — retract wins.
- `retract_transients` implementation: enumerate transient
  concepts via marker query, load each descriptor, walk its
  relation URIs, query `txn` for facts under those relations,
  retract each.
- End-to-end tests:
  - **Increment-counter**: submit a transient `increment`, the
    counter's value updates, the increment fact is absent from
    persisted state after settling.
  - **Mailbox-with-ack**: a persistent `message` exists, a
    transient `ack` is submitted, a `retract!: message` rule
    fires, the message is gone from persisted state.
  - **Cascade with transient intermediate**: rule A produces a
    transient command, rule B fires on that transient and
    produces persistent state. Both fire in the same commit
    cycle; the intermediate is not persisted.
  - **Silent drop**: a transient submission with no matching
    effect is dropped from the persisted delta.

### Phase 4 — ability discovery on subscription

- Subscription opens against concept `C`: also enumerate effects
  whose body has a positive premise reading a transient concept
  whose binding identifies `C` as a target.
- SSE frame extended to carry `abilities` alongside `conclusions`.
- Wire format frozen and documented.

### Phase 5 — UI for abilities (`<tonk-display>` / `<tonk-concept>`)

- `<tonk-display>` reads the ability list from the SSE stream
  and renders affordances (probably a button cluster). Click
  commits a fact submitting the transient ability concept.
- The fixpoint evaluator picks it up, fires the relevant rules.
  The subscription stream updates with the new state.

### Future work

V1 is complete enough to support local commands, mailbox
patterns (with persistent messages and transient acks), and
declarative state transitions. A few things that aren't in V1
but are within reach later:

- **Multi-head rules**: sugar for "this body produces both an
  assert and a retract." Currently expressed by two rules with
  identical bodies. Adding this is purely ergonomic.
- **Built-in `now` / clock**: enables expiry rules. Would
  require either an external clock-tick mechanism or special-
  casing in the evaluator. Deferred until concrete demand.
- **Cross-peer mailboxes**: persistent messages with a clear
  protocol for consumption across peers. Builds on V1's
  message-and-ack pattern but probably wants additional sugar
  (well-known retry/ack rules?).
- **Effects on persistent-only premises**: rules with no
  transient triggers, requiring either full re-evaluation on
  pull or stratification analysis. Deferred indefinitely;
  V1's transient-trigger requirement covers our use cases.

## Things to flag in the journal

- **Persistence is a rule.** Dialog's storage layer is the
  materialization of an implicit `p(X)@next :- p(X), notin
  retract(p, X)` rule applied to every EAV triple. The Dedalus
  reframing makes this visible and gives vocabulary for
  exceptions: transient concepts are concepts whose facts don't
  have the implicit rule applied.
- **Transient concepts are declared per-concept.** A concept
  marked `transient: true` produces facts that exist only at
  the timestep of submission. Mailboxes, commands, abilities,
  events all use this mechanism.
- **Two rule head polarities.** `assert!:` produces new facts;
  `retract!:` produces retracts. Each rule has one polarity.
  Multiple rules per head concept compose by disjunction (same
  as deductive rules) so adding a new event extends behavior
  without modifying existing rules.
- **Conflict resolution: change wins over no-change.** Retract
  beats assert on the same cell; both beat the implicit
  carry-forward.
- **Carry-forward is governed by retraction only.** A fact
  carries forward unless a retract is emitted on the same
  entity+attribute pair. Cardinality-one replacement is a
  storage-layer rule, not a carry-forward rule.
- **Effects are inductive rules over transients.** An effect's
  body must read at least one transient concept's facts.
  Consumption of the trigger is automatic by absence: nothing
  carries it forward.
- **Naming convention.** Internal Rust type is `InductiveRule`
  (upstream, sibling of `DeductiveRule`); tonk-yaml surface
  keyword is `effect!:`. The framework already named
  `dialog_effects` is a different layer (system IO surface) and
  we document the distinction rather than rename it.
- **No `ability` field**, inferred from the rule's body.
- **`effect:system` is convention, not mechanism.** Ability
  concepts conventionally use it as their `this` for ergonomic
  reasons (one well-known place to send commands), but the
  transactor doesn't special-case the URI. Transience comes
  from the concept's `transient: true` flag.
- **Pull doesn't fire effects.** Transients don't replicate, so
  the only way for an effect to fire is for the transient to be
  submitted locally. Pull is a graft of upstream changes onto
  the local tree; the reactor's `evaluate_effects` hook lives
  in `Commit::perform`, not `Pull::perform`. This preserves
  partial replication.
- **Effects are stored as concept facts** (`dialog.effect/*`),
  which means the "reverse index" needed by the evaluator is
  just a standing query, not a cached structure.
- **Reverse index is attribute-keyed.** Asking "which effects
  could be affected by a change to attribute X" is a single
  one-hop query. Concept-level invalidation falls out because
  attributes are what change in storage.
- **Chain reactions are handled by the semi-naive fixpoint
  loop** within a single commit cycle; subscribers see the
  settled state, not intermediate rounds.
- **Termination guarantee comes from the depth cap**, not from
  semantic reasoning. A pathological effect set will hit the
  bound and log a warning; stratification analysis is a
  possible later addition.

## Open questions still worth your call

- **`view` template syntax for triggering an ability.** The
  journal mentions `onclick={increment}` in view templates as a
  separate piece of UI plumbing. That's a parallel track — the
  effect itself doesn't care how the command fact gets asserted,
  only that one is asserted. The template-binding work is its
  own follow-on once Phase 4 lands.

## Reference

- `dialog-db/view/src/session.js` — the working JS prototype for the
  dispatch/expire/feedback loop. The `transact` and `propagate`
  methods are the closest analog to what `evaluate_effects` does.
- `dialog-db/view/src/view.js` — `fact(...).where(...).aggregate(...)`
  shows how view-style derivation rules are built; the inductive-rule
  body has the same shape.
- `dialog-db/worker/paul/rust/dialog-query/src/rule/deductive.rs` —
  the reference for compile-time analysis we reuse.
- `Dedalus` — the academic reference for inductive rules over
  time-stamped facts; the asserted-then-retracted-trigger pattern is
  Dedalus's `@next` semantics applied at commit granularity.
