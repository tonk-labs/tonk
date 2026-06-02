# Caller-chosen entity derivation

## Context

When an assertion omits `this:` and has no `&anchor`, we derive its
subject entity from the body — content-addressing the assertion so the
same payload always names the same entity. This is what lets a `view!:`
have a stable identity without the author inventing an id, and it's what
makes the notation path and the `/transact` wire path agree on which
entity a `(predicate, payload)` pair refers to.

This derivation is the right answer to the view-ambiguity problem. A
`view` concept is `{model, display}`, and nothing stops several views
being published for the same model; resolution then picks one
arbitrarily. If a view's identity is *derived from its fields*, two views
for the same model are simply two different entities, and there's no
collision to resolve. The same applies to any concept where "same content
means same thing."

But the derivation as it stands has two problems.

**It silently drops references.** The body digest includes only literal
scalars; variables, references, blanks, and nested forms are skipped. So a
view's `display` (a text literal) participates in its identity but its
`model` (an entity reference) does not. Two views that differ only by
model derive the *same* entity — the exact collision we were trying to
avoid. The digest should include everything by default, not a subset.

**The two paths disagree.** The wire path
(`application_plan_from_predicate`) derives from the *whole* resolved
parameter map, references included. The notation path (`body_digest`)
derives from literals only. The code claims these converge; they only
converge when the body has no references. A `view!: { model, display }`
written in notation and the same view applied over the wire derive
different entities today. That's a latent bug, not a feature.

The reason the notation digest skips references is sequencing, not design:
`body_digest` is a pure, resolver-free function so it can run in Phase 1
to pre-compute an anchor's entity before names are resolved. A reference
like `model: counter` is an unresolved symbol at that point. Skipping it
sidesteps the resolver — at the cost of correctness.

## What we want

Two things, in order of how the author experiences them:

1. **Implicit derivation includes every field.** When `this:` is omitted,
   the entity derives from *all* of the body's fields, with references
   resolved to the entities they name. No hardcoded subset. The notation
   path and the wire path converge by construction, because both hash the
   same resolved values.

2. **The caller chooses what identity means.** When `this:` is given as a
   field selection, the entity derives from exactly those fields — the
   instantiator decides the key, not the concept author and not the
   analyzer.

```yaml
# Implicit — identity is every field, references resolved.
view!:
  model: counter
  display: <span>{count}</span>

# Explicit — identity is just `model` (the field's resolved value).
# Re-edit `display` and the entity stays the same.
view!:
  this:
    model:
  model: counter
  display: <span>{count}</span>

# Explicit with a salt — `model`'s value plus a literal under `kind`.
view!:
  this:
    model:
    kind: "view"
  model: counter
  display: <span>{count}</span>
```

A bare key in `this:` (empty value) **selects** the like-named body field
and contributes its resolved value. A key with a value contributes that
literal directly, under that name — a salt or a rename. The existing
forms of `this:` are unchanged: omitted → all fields; `?var` → a binding;
a uri / bare symbol → that exact entity (no derivation).

## Where the work lands

Analyzer-only on the surface; the parser already produces what we need.

- **`derive_head_intent`** rejects `FieldValue::Nested` in `this:` today —
  that's the one rejection to lift. The parser already hands us the nested
  object; we interpret it instead of erroring. This yields a new
  `ThisIntent` variant carrying the field selection (the selected names
  plus any literal salts).

- **`body_digest`** stops filtering by `FieldValue` kind. By default it
  takes every field; in the explicit case it takes the selected subset
  plus salts. Either way it must resolve references (`model: counter` →
  the concept entity) before hashing, which means it can no longer be the
  pure Phase-1 function it is now. Derivation moves to where the scope /
  resolver is available — the wire path already proves the recipe works
  post-resolution, so this is aligning the notation path to it, not
  inventing anything.

- **`application_plan_from_predicate` / `derive_this`** is the convergence
  target and needs no recipe change — it already hashes the full resolved
  payload. Once the notation path resolves references and hashes the same
  values, the two agree. The explicit-selection case needs the wire path
  to honor the same subset, so the selection has to ride along into the
  wire payload (or be applied identically on both sides).

- **Phase ordering.** The one real consequence: an anchor's derived entity
  can't be known until its references resolve. Anywhere Phase 1
  pre-computes an anchor entity from `body_digest` has to tolerate that
  derivation now depends on resolution. Worth confirming during
  implementation whether any in-doc forward reference relies on the
  Phase-1 pre-computation.

## Pinning a concept's entity (`this:` on a `concept!`)

A concept's entity is content-derived from its descriptor by default, but
a `this: <uri>` on a `concept!` declaration **pins** it to a stable, chosen
entity (`tonk:view`, `tonk:artifact`, …) so the concept stays referenceable
by that URI even if its published name later moves. This is intended
behavior the analyzer wasn't implementing — the same shape of bug as the
digest: `parse_concept_body` dropped `this:` and always used
`descriptor.this()`.

The mechanism already existed: built-in concepts pin themselves to
`db:<name>` via `ConceptDefinition.entity` (a slot carried separately from
the descriptor). The fix makes user concepts use the same slot:

- `parse_concept_body` honors a `this: <uri>` (URI → pin; `?var` / omitted
  → derive, preserving the existing variable-binding behavior).
- Instance derivation carries the concept's resolved entity
  (`resolved.entity` / `def.entity`) instead of recomputing
  `descriptor.this()` — so a pinned concept's instances derive from the
  pinned URI, and notation/wire converge.

**Wire convergence.** The wire `ConceptDescriptor` carries no pin slot, so
`application_plan_from_predicate` can only fall back to `descriptor.this()`
when `this` is absent. In practice this never diverges: every programmatic
wire producer (`lower_statement`, hence `claim!`/bootstrap) carries `this`
explicitly in the payload, which makes the wire path skip derivation
entirely. The only way to hit a divergence is a hand-rolled `/transact`
caller that sends a pinned concept's descriptor *and* omits `this` — an
unreachable edge today. If ever needed, add an optional pinned-entity field
to the wire `ConceptDescriptor` and have `application_plan_from_predicate`
prefer it. Locked down by `it_converges_wire_path_for_pinned_concept_when_this_is_carried`.

## PR sequence

### PR 1 — fix the digest (do this first)

Make the omitted-`this:` case derive from every field with references
resolved, and fold in the path convergence: drop the literals-only filter,
resolve references, and align the notation derivation with
`application_plan_from_predicate`. Add a test that a `view!:` with a
`model` reference derives the *same* entity through notation and through
the wire path. After this PR, two views for the same model with different
bodies are distinct entities, and two with identical bodies coincide — on
both paths. This is the bug fix and it stands alone; the explicit `this:`
surface below is a separate follow-up.

### PR 2 — explicit `this:` field selection + salt (follow-up)

Deferred until PR 1 lands. Accept `FieldValue::Nested` in `this:`: a new
`ThisIntent` carrying the selected field names and literal salts. Bare key
selects the body field's resolved value; valued key contributes a literal.
Ensure the selection reaches the wire path so command-applied and
notation-written assertions still converge. After this PR a view can be
keyed by `{model}` alone (or `{model, kind:"view"}`), so editing its
template preserves its identity.

## Open questions

- **Forward references and Phase 1.** Does any current in-document pattern
  depend on an anchor's entity being known before its references resolve?
  If so, those derivations now have to wait for resolution. Confirm the
  blast radius during PR 1.

- **Selection over a reference that doesn't resolve.** `this: { model: }`
  where `model` is absent or unresolved — error loudly (it changes
  identity silently otherwise). Decide the exact diagnostic in PR 2.

- **Carrying the selection over the wire.** Whether the field selection
  travels in the predicate application payload or is recomputed identically
  on both sides. Either works as long as the two paths can't drift; settle
  in PR 2.
