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

### Two ideas to keep separate

**Inductive rules** are the general concept (defined upstream).
Their full semantic obligation is "re-fire whenever any state they
read could have changed" — which, in a replicated system, means
re-firing on every pull too. That's expensive and breaks partial
replication (the runtime would have to materialize subtrees it
deliberately didn't replicate, just to ask "do any of my rules
mention concepts that changed").

**Effects** are the subset of inductive rules tonk supports in
this initial release: rules that read at least one premise from
`effect:system`, an ephemeral sentinel entity whose facts never
persist. Because the trigger never replicates, an effect can only
fire on the peer that originated the trigger, and the *output*
(durable head facts) replicates to other peers as plain state. No
post-pull re-evaluation needed; partial replication preserved.

This is V1. V2 (mailbox-shaped rules — read from a known concept
that the runtime scans on pull and replays as new commits) and V3
(full re-evaluation on pull for rules that fit neither pattern)
are future work; the V1 restriction is a temporary optimization
gate, not a semantic claim about what inductive rules can be.

> [!note]
>
> *Naming.* The word "effects" is overloaded with `dialog_effects`
> (the framework for system IO surface — archive, authority,
> memory handlers). Those are operational effects produced by
> handlers; the effects in this document are declarative
> state-transition specs. Same word, different layer. Internal
> Rust type is `InductiveRule` (upstream); the tonk-yaml surface
> keyword is `effect!:` for the V1-shaped subset.

## What an effect looks like

In dialog-yaml, the surface authors write:

```yaml
# A command concept. Asserting one of these on `effect:system`
# is how a user "invokes the increment ability against ?counter".
concept!: &increment
  description: Command to increment a counter
  with:
    subject:
      the: tonk.xyz.command/subject
      as: entity

# An effect: when an `increment` command targets a counter that
# already has a count, derive a new counter row with count+1.
# The `where: { this: effect, ... }` clause on the trigger
# premise tells the engine "this is the ability that fires me."
effect!:
  description: Increment a counter on increment command
  assert!: counter
  where:
    this: ?this
    count: ?next-count
  when:
    - assert: counter
      where:
        this: ?this
        count: ?last-count
    - assert: increment
      where:
        this: effect:system
        subject: ?this
    - assert: +
      of: ?last-count
      with: 1
      is: ?next-count
  unless:
    - assert: disable
      where:
        this: effect:system
        ability: counter
```

Roughly: think of `effect!:` as `DeductiveRuleDescriptor` extended with
**fire semantics**: when the `when`/`unless` body produces bindings,
the engine *commits* the head as new facts (instead of just yielding a
tuple), and retracts the ability fact that triggered the firing.

`effect:system` is a sentinel entity (a known DID) meaning "the
ambient command bus." Users assert commands against it; effects
match on it; on firing, those facts never make it into the
persisted branch state. This is more scalable than the
per-effect-entity scheme in the original sketch (one bus, many
concurrent commands).

### `effect:system` — the ambient command bus

`effect:system` is a well-known sentinel entity. Think of it as
the rough equivalent of stdin or the (deprecated) `window.event`:
an ambient stream of commands handlers read but no one stores.

Assertions on `effect:system` are **commit-scoped**: they exist
only for the duration of one `evaluate_effects` cycle. The
trigger arrives in the incoming transaction, the effect loop
reads it across however many fixpoint rounds, then those facts
are stripped from the delta before the branch state is written.
No separate retract instruction — the fact simply never enters
durable storage.

Three consequences:

- **Triggers don't replicate.** Because they never persist, they
  never end up in the upstream tree, never get pushed, never get
  pulled. This is the property that makes V1 effects safe under
  partial replication — only the peer that originated the trigger
  ever sees it.
- **A trigger that matches no effect is silently dropped.** An
  unanswered command does not sit in storage. The semantic is
  "asserting on `effect:system` is sending a message; the message
  lifetime is one commit cycle."
- **No leftover triggers from a crash.** A crash mid-cycle drops
  the whole transaction; nothing partially-persisted to recover.

### V1 restriction (effects vs. general inductive rules)

For this initial release, an `effect!:` rule must include **at
least one positive `when` premise reading from `effect:system`**.
This is checked at effect-registration time (tonk-side, not
upstream — `InductiveRule::new` itself accepts any well-formed
inductive rule).

Rules that don't satisfy this restriction are valid inductive
rules upstream, but tonk's reactor refuses to install them as
effects. Two reasons:

1. **Convergence under partial replication.** A rule that reads
   only durable concepts would need to re-fire on every pull to
   stay in sync with remote facts. With partial replication, the
   peer can't even see the changes that would need to trigger the
   rule.
2. **Bounded work.** Even with full replication, "fire all rules
   on every pull" scales poorly. V1 effects skip the pull
   evaluation entirely; future versions (V2 mailbox, V3 full
   re-fire) trade more work for broader rule shapes.

The error message points at the missing trigger and suggests
adding `assert: <some-command>, where: { this: effect:system,
... }`. Authors who want a derived view of state should use a
deductive rule (computed on query); authors who want a state
transition triggered by a local command use an effect.

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

There is **no** `dialog.effect/ability` field. The ability is
*inferred*: any positive premise of shape `assert: X, where: {
this: effect:system, ... }` exposes `X` as an ability whose
subject is the binding for `?this`. (The V1 restriction means
every stored effect has at least one such premise — see Layer 3.)

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
   premise must read from `effect:system`. Reject (with a
   diagnostic fact) if not.

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

**The loop.** Each round computes the set of concepts whose facts
changed, queries for the candidate effects, fires what matches.
The `effect:system` working set is held separately and never
persisted; only the head assertions and any non-`effect:system`
side effects of those firings land in the durable delta.

```rust
fn evaluate_effects(branch, initial_txn) {
    const MAX_DEPTH: u32 = 16;
    let mut dirty: HashSet<ConceptId> = concepts_touched(&initial_txn);
    let mut ephemeral = initial_txn.filter(|f| f.this == EFFECT_SYSTEM);
    let mut persistable = Transaction::new();
    let mut depth = 0;

    loop {
        if dirty.is_empty() || depth >= MAX_DEPTH { break; }
        // Standing query: which effects could be triggered by these concepts?
        let candidates = query_effects_by_premise_concepts(branch, &dirty);
        let mut delta = Transaction::new();

        for effect_id in candidates {
            let effect = branch.load_effect(effect_id);
            if any_unless_matches(branch, &ephemeral, &effect) { continue; }
            let bindings = run_query(branch, &ephemeral, &effect.join);
            for tuple in bindings {
                delta.merge(effect.assertion.instantiate(&tuple));
            }
        }

        if delta.is_empty() { break; }
        if depth + 1 >= MAX_DEPTH {
            log_runaway_effects(&candidates, depth, &delta);
        }

        // Split: facts on effect:system are ephemeral (next round only);
        // everything else is persistable.
        let (next_ephemeral, durable) = delta.partition(|f| f.this == EFFECT_SYSTEM);
        persistable.merge(durable.clone());
        branch.apply(&durable);
        ephemeral = next_ephemeral;
        dirty = concepts_touched(&delta);
        depth += 1;
    }

    // Original effect:system facts in initial_txn are dropped here —
    // they only live for the duration of this loop.
    persistable
}
```

There is **no cold-start round.** Inductive rules with command
triggers can't have pre-existing triggers to drain because
`effect:system` facts never persist. (Inductive rules without
command triggers — pure derivations — are out of scope for v1;
that's what `DeductiveRule` already handles via query-time
materialization.)

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
    if premise matches `assert: X, where: { this: effect:system, subject: ?v, ... }`:
        let target_concept = head_concept_for_var(effect, ?v)
        if target_concept == C:
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

### Phase 2 — effect storage

- Schema constants for
  `dialog.effect/{assert,premise,unless,description}` in
  `tonk-schema`.
- `load_effects(branch)` that reconstructs `InductiveRuleDescriptor`
  from facts, compiles via the upstream `Compile::compile`, and
  validates the V1 restriction (at least one positive
  `effect:system` premise).
- Failed-validation surface: `dialog.effect/error` triple (or
  equivalent diagnostic concept) so the LSP can show the failure.
- `effect:system` URI constant — well-known DID, documented as
  the ambient command bus.

### Phase 3 — semi-naive fixpoint evaluator

- A standing query: "effects whose `dialog.effect/premise`
  references concept `X`," used per round by the loop. Both
  `when` and `unless` premises participate.
- `evaluate_effects` step in the reactor between
  `apply_local_txn` and `notify_subscribers`, driven by that
  query. **Not** hooked into `Pull::perform`. V1 effects' only
  durable change is the head; the trigger never replicates so
  there's nothing for pull to fire.
- The ephemeral / persistable split on `effect:system` facts:
  assertions arriving in the transaction or produced by effect
  firings live for the duration of the loop only; everything
  else is folded back into the persistable delta.
- Single-round implementation first (depth=1), with a hard-coded
  test rule that increments a counter on an `increment` command.
- Add the fixpoint loop with MAX_DEPTH=16 and a structured
  warning on hitting the bound.
- End-to-end tests:
  - Assert an `increment` command; the counter increments and
    the command fact is absent from persisted state after the
    commit settles.
  - The `counter → counter-snapshot → alert` cascade reaches
    `alert` within MAX_DEPTH rounds with one external
    notification (cascade still works through `effect:system`
    intermediates).
  - An `effect:system` fact that matches no effect is silently
    dropped from the persisted delta.

### Phase 4 — ability discovery on subscription

- Subscription opens against concept `C`: also enumerate effects
  whose trigger asserts an ability whose subject binds to `C`.
- SSE frame extended to carry `abilities` alongside `conclusions`.
- Wire format frozen and documented.

### Phase 5 — UI for abilities (`<tonk-display>` / `<tonk-concept>`)

- `<tonk-display>` reads the ability list from the SSE stream
  and renders affordances (probably a button cluster). Click
  commits a fact asserting `<ability-concept>` with `{ this:
  effect:system, subject: <host-entity> }`.
- The fixpoint evaluator picks it up, fires, drops the trigger.
  The subscription stream updates with the new state.

### Future work — V2 and V3

V1 covers the "local command triggers durable state change" use
case. Two follow-ups, deferred until concrete motivation:

- **V2: mailbox-shaped rules.** Inductive rules whose body reads
  from a known mailbox concept (origin-keyed or shared) can
  participate in cross-peer messaging. Pull integration scans
  the bounded mailbox key range, finds new messages, replays
  them as a local commit (which then fires the relevant
  effects). Partial replication preserved because the scan is
  bounded by mailbox key prefix, not by the size of the pull
  delta.
- **V3: full re-evaluation on pull.** For inductive rules that
  fit neither the V1 nor V2 pattern, the runtime re-fires all
  rules on every pull. Correct but expensive; recommended only
  when other patterns don't apply, with documentation warning
  about the cost. Incompatible with partial replication unless
  the runtime can prove the rule's body is satisfied entirely by
  the locally-replicated subtree.

Each version layers on top of its predecessor — V2 keeps V1's
local triggers; V3 adds general re-evaluation as a fallback. The
authoring surface gains progressively richer rule shapes; the
optimization gate moves accordingly.

## Things to flag in the journal

- **Inductive rules vs. effects.** Two layered ideas. Inductive
  rules are the general concept (asserts head when body matches,
  semantically obligated to re-fire whenever any input could
  change). Effects are the V1-restricted subset tonk supports:
  rules that include a positive `effect:system` premise. The
  restriction is a temporary optimization gate so we can skip
  pull-time re-evaluation; future versions broaden the
  accepted shapes.
- **Naming convention.** Internal Rust type is `InductiveRule`
  (upstream, sibling of `DeductiveRule`); tonk-yaml surface
  keyword is `effect!:`. The framework already named
  `dialog_effects` is a different layer (system IO surface) and
  we document the distinction rather than rename it.
- **No `ability` field**, inferred from `where: { this:
  effect:system, ... }` shape.
- **`effect:system` as the command bus.** One known DID; all
  facts asserted on it are commit-scoped and never persisted.
  No separate retract mechanism — facts simply don't enter
  durable storage. An unanswered command is silently dropped.
  The mental model is stdin or `window.event`: an ambient
  command stream handlers read but no one stores.
- **Pull doesn't fire effects.** Triggers don't replicate, so
  the only way for an effect to fire is for the trigger to be
  asserted locally. Pull is a graft of upstream changes onto the
  local tree; the reactor's `evaluate_effects` hook lives in
  `Commit::perform`, not `Pull::perform`. This preserves partial
  replication — the runtime never has to materialize subtrees it
  didn't replicate just to ask whether some rule might fire.
- **Effects are stored as concept facts** (`dialog.effect/*`),
  which means the "reverse index" needed by the evaluator is
  just a standing query, not a cached structure. No
  invalidation, no cold-start refresh, no separate lifecycle.
- **Chain reactions are handled by the semi-naive fixpoint
  loop** within a single commit cycle; subscribers see the
  settled state, not intermediate rounds.
- **Demand-driven evaluation via the standing query** means
  cost scales with what changed, not with the size of the effect
  set.
- **Termination guarantee comes from the depth cap**, not from
  semantic reasoning. A pathological effect set will hit the
  bound and log a warning; stratification analysis is a possible
  later addition.

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
