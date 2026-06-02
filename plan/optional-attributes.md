# Optional concept attributes — integration plan

## Status: implemented

All phases landed; `cargo fmt`, `cargo clippy --all --all-targets --all-features -D warnings`, and `cargo test --workspace` are green. Notable deviations from the plan below, discovered against the real branch source (`5ddee71`):

- **Constructors:** dialog kept fallible `TryFrom<Vec<(_, AttributeDescriptor)>>` impls that auto-wrap as required, so most sites were just `from` → `try_from` (no manual `ConceptFieldDescriptor::required`). Empty-set stubs (`stub_predicate`, the `realize` synthetic predicate, the all-`this` domain application) had to gain a placeholder field, since an empty descriptor is now rejected.
- **Phase 4 (evaluator) needed no code change for set-widening.** Optional resolution is derived by dialog's planner from the descriptor's per-field `is_optional()` flag, and tonk runs concept queries through dialog's `ConceptQuery::evaluate`. So once the descriptor carries the flag (in-doc parse + branch reconstruction), the engine omits absent optionals automatically. The only evaluator-side work was adapting to `Match::lookup` now returning `Binding` (Present/Absent) instead of `Value` — absent bindings are dropped from frames/conclusions.
- **Derive change:** the `#[derive(Concept)]` macro no longer emits `From<Query> for ConceptDescriptor`; the descriptor is read via `Descriptor<ConceptDescriptor>::descriptor()` on the struct type. `builtin::<Q>` was retargeted from the `Query` newtypes to the struct types.
- **Storage marker domain:** optionality is stored as a boolean marker `dialog.concept.optional/{field}` (sibling to the existing `dialog.concept.with/{field}` link), replacing the obsolete `dialog.concept.maybe/*`. Required-field storage is unchanged.
- **Pre-existing test fixed:** `it_lifts_retract_polarity` had a latent type conflict (`?this` bound to a text field in one premise, an entity in another) that the PR's new rule-level type inference correctly rejects; the fixture's `target` field was changed to entity-typed.

## Test coverage

Added tests, by behavior:

- **Parse / declaration** (`tonk-analyzer/src/analyzer.rs`): `maybe:` block marks fields optional and emits the `optional.{field}` marker; all-optional concept rejected (`InvalidConceptBody`); duplicate field across `with:`/`maybe:` rejected (`E_DUPLICATE_CONCEPT_FIELD`); a `maybe:` **bare reference** (not just inline) is marked optional.
- **Assertion completeness** (`tonk-analyzer/src/analyzer.rs`): omitting an optional field on a fresh entity succeeds; omitting a required field still raises `IncompleteAssertion` listing only the required field. A new `concept_typed_optional` fixture registers optional-bearing concepts on the branch.
- **Query set-widening, end to end** (`tonk-evaluator/src/evaluate.rs`): a `concept:` query returns both an entity that has the optional field (field present) and one that lacks it (field omitted), proving the field-less entity isn't filtered out.
- **Storage round-trip** (`tonk-evaluator` + `tonk-schema`): the `dialog.concept.optional/{field}` marker is persisted for optional fields only; `ConceptByEntity::resolve` rebuilds a descriptor whose `is_optional()` matches per field.
- **Relation helpers** (`tonk-schema/src/concept.rs`): `optional()` / `parse_optional()` round-trip and reject the wrong domain.

Known remaining gaps (low risk, not blocking):

- **Head-fact emission with an Absent optional operand** (`effects.rs` `fire_effect`): the omit-on-Absent guard is in place and exercised by the same `Binding::as_value()` path the query test covers, but there's no dedicated inductive-rule test where a head's optional field is Absent for some matches. Worth adding if optional fields start appearing in rule heads.
- **Serialized shape / backward-compat byte-identity:** that a required-only descriptor serializes byte-identically to pre-optionality, and that an optional field serializes with `optional: true`, is covered upstream in `dialog_query` but not re-asserted here.
- **LSP hover cosmetics:** `render_concept_hover` renders `field?` + `(optional)`; no direct unit test (pure formatting).

## Goal

Adopt dialog-db [PR #346][pr] (`feat/type-inference-v2`) so a concept can declare attributes as **optional**. Once integrated:

- A concept author marks an attribute optional in the `with:` block.
- The analyzer & LSP stop demanding optional attributes on assertions.
- Queries omit optional attributes on entities that lack them (set-widening / left-join), instead of excluding those entities.

[pr]: https://github.com/dialog-db/dialog-db/pull/346

## What the upstream PR actually changes (and why it matters here)

The PR does **not** keep the old separate `maybe:` map. It **consolidates** optionality into the single `with` map, one flag per field:

```rust
// dialog-query, feat/type-inference-v2
pub struct ConceptDescriptor {
    description: Option<String>,
    with: NamedAttributes,            // BTreeMap<String, ConceptFieldDescriptor>
}

pub struct ConceptFieldDescriptor {
    #[serde(flatten)]
    descriptor: AttributeDescriptor,
    #[serde(default, skip_serializing_if = "is_not_optional")]
    optional: bool,                   // wire: `"optional": true`, omitted when false
}

impl ConceptFieldDescriptor {
    pub fn required(d: AttributeDescriptor) -> Self;
    pub fn optional(d: AttributeDescriptor) -> Self;
    pub fn is_optional(&self) -> bool;
    pub fn descriptor(&self) -> &AttributeDescriptor;
    pub fn the(&self) -> &The;
    pub fn to_uri(&self) -> String;
}

// NamedAttributes::iter() now yields (&str, &ConceptFieldDescriptor)
```

Plus on the query side:

```rust
pub enum Resolution { #[default] Required, Optional }  // serde lowercase
// AttributeQuery gains resolution()/with_type(); with_is() is removed.
// An optional field becomes a set-widened AttributeQuery: on a miss it
// yields one fallback row with `is` bound to Absent, instead of zero rows.
```

Identity is preserved: a concept with no optional fields hashes exactly as before, so existing `concept:` entity URIs do not change.

### Consequences for this repo

1. **`descriptor.with().iter()` now yields `ConceptFieldDescriptor`, not `AttributeDescriptor`.** Every iteration site must switch from `attr.content_type()` / `attr.to_uri()` to `attr.descriptor().content_type()` / `attr.to_uri()` (the wrapper forwards `to_uri`/`the`) and may read `attr.is_optional()`. This is a **mechanical but repo-wide** change — ~40 call sites across analyzer, evaluator, schema, language-server, slide.

2. **`ConceptDescriptor::from(Vec<(String, AttributeDescriptor)>)` is gone**, replaced by fallible `try_from(Vec<(String, ConceptFieldDescriptor)>)` that rejects empty/all-optional field sets. Every constructor site must wrap descriptors in `ConceptFieldDescriptor::required(...)` / `::optional(...)` and handle the `Result`.

3. **The old `maybe:` machinery is obsolete.** `tonk_schema::concept::{maybe, parse_maybe, MAYBE_DOMAIN}` and the `dialog.concept.maybe/*` storage domain are no longer the model. Optionality now rides as a per-field flag, stored alongside the existing `dialog.concept.with/{field}` claim (see Storage round-trip below). We remove the dead `maybe`/`parse_maybe` helpers.

4. **The JSON-builder browser path is mostly unaffected on the wire** (the `optional` key is flattened into each field and `skip`ped when false), but anything that *reads* optionality from that JSON must look for the `optional` key.

## Notation syntax (decided)

A concept body has two **symmetric** field blocks: `with:` for required fields and `maybe:` for optional fields. They parse identically (bare references and inline definitions both allowed); the only difference is that `maybe:` implies `optional = true` on every field it contains. There is no per-field `optional:` marker.

```
concept!
  with:
    name: person-name        # required (bare reference)
    age:                     # required (inline definition)
      as: unsigned-integer
  maybe:
    nickname: person-nick    # optional (bare reference)
    bio:                     # optional (inline definition)
      as: text
```

Semantics:

- `with:` and `maybe:` accept exactly the same shapes (bare symbol ref, `?var`, URI, or inline attribute def). The block chooses required vs. optional; nothing inside a field marks optionality.
- Both blocks merge into the single upstream `ConceptDescriptor.with` map, each field wrapped as `ConceptFieldDescriptor::required(..)` (from `with:`) or `::optional(..)` (from `maybe:`). `maybe:` is notation/sugar only — not a separate stored map.
- A field name must not appear in both blocks, nor twice within one block. This is a hard error surfaced by both the analyzer and the LSP (see Phase 2).
- `maybe:` is optional and may be omitted. `with:` must declare at least one field — an all-optional concept (no `with:`, only `maybe:`) is a parse error, matching upstream `TypeError::EmptyConcept`.
- These blocks only carry meaning in a `concept!` body. A standalone `attribute!` declaration has no notion of optionality.

## Storage round-trip (branch persistence)

A concept persisted on a branch records one `dialog.concept.with/{field}` claim per field (pointing the concept entity at the attribute entity). Optionality must survive this round-trip so `ConceptByEntity::resolve` can rebuild a faithful descriptor.

**Approach:** emit a sibling marker claim for optional fields,
`dialog.concept.optional/{field}` → `true`, written only when the field is optional. On reconstruction, collect the set of optional field names first, then build each `ConceptFieldDescriptor` as `optional(..)` or `required(..)` accordingly. This keeps required-field storage byte-identical to today (no marker), so existing branches are unaffected, and it does not perturb concept identity (identity is the descriptor hash, computed from `with` + the per-field optional flag — see upstream note that this is value-object-preserving).

This replaces the never-emitted `dialog.concept.maybe/*` path.

## Work breakdown

### Phase 0 — point deps at the branch, get it compiling

1. In `Cargo.toml` change all `dialog-*` deps from `tag = "tonk-2026-05-28"` to `branch = "feat/type-inference-v2"` (lines 140–153). Update `Cargo.lock` (`cargo update -p dialog-query …` or a plain build).
2. Build the workspace. Expect a wall of type errors at every `descriptor.with().iter()` and `ConceptDescriptor::from(...)` site. Triage into the mechanical fixes below.

### Phase 1 — mechanical API migration (no behavior change yet)

Treat every field as **required** so behavior is unchanged; this isolates the API churn from the feature work.

- **Iteration sites** — change `for (name, attr) in d.with().iter()` bodies to use `attr.descriptor()` where an `&AttributeDescriptor` is needed, and `attr.to_uri()` (forwarded) where a URI is needed. Sites (from research):
  - `tonk-analyzer`: `analyzer/declaration.rs:397,465`, `analyzer/assertion.rs:155,242,574`, `analyzer/query.rs:43,71`, `analyzer/rule.rs:324,580,605`, `analyzer/graph.rs:340`.
  - `tonk-evaluator`: `evaluate.rs:711,763,982`, `effects.rs:563,592,631,769`.
  - `tonk-schema`: `concept.rs` emit/resolve (`emit_concept_facts` ~1271, `resolve` ~263), `transact.rs:289-314`, `rule_query.rs`, `builtin.rs`, `resolution.rs`.
  - `tonk-language-server`: `server.rs:794,983`.
  - `slide`: `schema.rs:340`, `views.rs`.
- **Constructor sites** — replace `ConceptDescriptor::from(fields)` with `ConceptDescriptor::try_from(fields.into_iter().map(|(n,d)| (n, ConceptFieldDescriptor::required(d))).collect())?` and thread the `Result`/`expect` as appropriate:
  - `tonk-schema`: `concept.rs:287` (resolve), `transact.rs:314`, `rule_query.rs`.
  - `tonk-analyzer`: `declaration.rs` (the `serde_json::from_value::<ConceptDescriptor>` path keeps working as-is — serde handles the new shape — but verify the `with` JSON it builds round-trips; required fields serialize without `optional`).
  - test fixtures in `tonk-evaluator` (`effects.rs:1473`, `evaluate.rs:1029`) and `slide/tests`.
- **`AttributeQuery` sites** — `with_is` is removed but a grep shows this repo never calls `with_is`; the ~50 `AttributeQuery::new/from` sites use the term-builder form (`.of(..).is(..)`) which is unchanged. Confirm `Resolution` defaults to `Required` so these stay required.
- **Remove dead `maybe`/`parse_maybe`** in `tonk-schema/src/concept.rs` (and their tests at ~1333). Leave `with`/`parse_with`.

Gate: `cargo build --all` + existing test suite green, behavior identical to `tonk-2026-05-28`.

### Phase 2 — notation: parse & declare optionals

- `tonk-analyzer/src/analyzer/declaration.rs`:
  - Factor the existing `with:` field-parsing loop (~199-237) into a helper that parses a block of fields (bare refs + inline defs) into `Vec<(String, AttributeDefinition)>`, parameterized by `optional: bool`. Call it once for `with:` (`optional = false`) and once for `maybe:` (`optional = true`).
  - `parse_concept_body` (~192): recognize a `"maybe"` field name alongside `"with"`. Both are nested mappings; anything else stays `UnknownField`. `maybe:` is allowed to be absent; `with:` must be present and non-empty (keep the existing "`with:` is required" check, now phrased as "at least one *required* field").
  - **Duplicate-field detection (analyzer + LSP).** While merging the two blocks, track seen field names in a map keyed on name, retaining each field's `name_range`. On a collision (same name in both blocks, or repeated within one block) emit a new `AnalyzeErrorKind::DuplicateConceptField { concept, field, first_range }` (code `E_DUPLICATE_CONCEPT_FIELD`) anchored at the *second* occurrence's `name_range`. Analyzer `AnalyzeError`s already flow to the language server as LSP diagnostics keyed by code, so the editor picks it up automatically; add an LSP test asserting it surfaces with the right code/range. (The top-level `E_DUPLICATE_NAME` is for declaration names — not appropriate here.)
  - Carry the optional flag on each parsed field (extend the `(String, AttributeDefinition)` tuple to `(String, AttributeDefinition, bool)` or a small struct).
  - `parse_concept_body` JSON build (~245-263): inject `"optional": true` into each `maybe:` field's object before `serde_json::from_value::<ConceptDescriptor>` (serde understands the flattened key). Keep the explicit "at least one required field" check so the user gets a clean notation diagnostic, never a raw serde `TypeError::EmptyConcept`.
  - `concept_schema` / `concept_application` (~381-509): emit the `dialog.concept.optional/{name}` marker claim for fields where `attr.is_optional()`.
- `tonk-notation`: the AST is name-agnostic, so no grammar change is needed — both `with:` and `maybe:` arrive as ordinary nested `Field`s. Add parse tests covering a `maybe:` block (bare ref + inline) and the duplicate-name case.

### Phase 3 — analyzer: stop requiring optionals

- `analyzer/assertion.rs`:
  - `check_complete_when_unbound` (~574): skip optional fields when computing `missing` — `for (name, attr) in d.with().iter() { if attr.is_optional() { continue } … }`. An omitted optional must not trip `IncompleteAssertion`.
  - main field walk (~155): optional fields the user omitted should **not** be forced to a blank that triggers retraction logic; leave them absent. Verify retract-side handling treats absent-optional as "nothing to retract."
- `analyzer/query.rs` (~43-62): this is the key match-side change. For optional fields the user omitted, do **not** inject `Term::var(field_name)` as a required conjunct. Instead emit the field as an **optional** match so entities lacking it still match (set-widened). Concretely, build that field's `AttributeQuery` with `Resolution::Optional` (the `ConceptQuery`/term plumbing must carry an optional marker through to evaluation — see Phase 4). The `UnknownField` check (~71) already iterates `with()`, so optional fields remain referenceable; no change needed there beyond the wrapper accessor.
- `analyzer/error.rs`: `IncompleteAssertion.missing` now means required-only; update its message if it implies "all fields." Add the `DuplicateConceptField { concept, field, first_range }` variant + `E_DUPLICATE_CONCEPT_FIELD` code (used by Phase 2's duplicate detection).

### Phase 4 — evaluator: omit optionals on non-matching entities

- `tonk-evaluator/src/evaluate.rs`:
  - Query expansion (`~711`, and `render_one_result` ~982): when a `ConceptQuery` field is optional, build its `AttributeQuery` with `Resolution::Optional` so a missing fact yields an `Absent` row rather than dropping the entity. The render path already `continue`s on unbound terms, so an `Absent`/absent optional simply does not appear in the result fields — confirm `Absent` is treated as "omit from output," not as a literal value.
  - Head-fact emission in `effects.rs` (`accumulate_head_facts`/`emit_head_facts_into` ~555-646) already `continue`s on missing terms and is tolerant; verify an optional field bound to `Absent` is skipped (not emitted as a fact).
- The `ConceptQuery` IR (in `tonk-schema`) must carry per-field optionality from analyzer to evaluator. Two options:
  - (a) re-derive from `predicate.with()[field].is_optional()` at evaluation time (no IR change, simplest), or
  - (b) thread `Resolution` on the term. Prefer (a): the descriptor is already on `ConceptQuery.predicate`, so the evaluator can ask `is_optional()` directly when expanding each field. This avoids IR/serde changes.

### Phase 5 — storage round-trip

- `tonk-schema/src/concept.rs`:
  - `emit_concept_facts` (~1271): after the `dialog.concept.with/{field}` claim, emit `dialog.concept.optional/{field}` → `Value` truthy marker when `attr.is_optional()`.
  - `ConceptByEntity::resolve` (~240) and `resolve_branch_descriptor` (~1083): first pass collects optional field names from `dialog.concept.optional/*` claims; second pass builds each `ConceptFieldDescriptor::{optional,required}` and `ConceptDescriptor::try_from(...)`.
  - Add a `concept::optional(field)` / `parse_optional(the)` helper pair (mirroring `with`/`parse_with`); delete `maybe`/`parse_maybe`.
- `tonk-schema/bootstrap.yaml`: the concept-of-concept schema currently reserves a `maybe..:` field. Replace/repoint it so the bootstrap descriptor matches the new model (or drop it if optionality is carried purely as the per-field marker). Verify bootstrap still loads.

### Phase 6 — LSP / hover & browser projection

- `tonk-language-server/src/server.rs` (`render_concept_hover` ~977): render optional fields, e.g. annotate them `name?: text` or `(optional)`, reading `attr.is_optional()`.
- Browser/JSON path — `tonk-concept/src/resolve.rs::phase2_query` (~137), `tonk-display`, `tonk-ui`, `tonk-layout` predicate builders: today they read only `with`. With the consolidated model they already see optional fields inside `with` (good), but a *query* built from such a predicate must not require optional fields. Where these build match terms from `with`, skip/relax fields whose JSON carries `"optional": true`. Audit each builder; this is the browser analogue of Phase 4.

### Phase 7 — tests & docs

- Notation parse tests: `maybe:` block accepted (bare ref + inline); concept with only `maybe:` (no `with:`) rejected; field in both `with:` and `maybe:` (and twice in one block) raises `E_DUPLICATE_CONCEPT_FIELD` at the second occurrence; an LSP test asserts that diagnostic surfaces with the right code and range.
- Analyzer tests: assertion omitting an optional field does **not** raise `IncompleteAssertion`; omitting a required field still does.
- Evaluator tests: querying a concept returns entities that lack an optional attribute, with that field omitted from results; entities that have it include it.
- Round-trip test: define → persist → `ConceptByEntity::resolve` preserves the optional flag.
- Use `#[dialog_common::test]` and `it_<verb>` naming per repo convention; ensure wasm test modules keep `run_in_browser`.
- Update `tonk-schema/design.md` (drops `maybe`, documents the per-field flag + `dialog.concept.optional/*`).

## Risks / open questions

- **Branch is a moving target.** Pinning to `branch = …` means upstream force-pushes can break our build; once merged, repin to a tag/commit.
- **Engine support for `Resolution::Optional` in the actual query path** must be real end-to-end (the PR claims set-widening in `only.rs`/`all.rs`). Phase 4 depends on it; validate with a focused evaluator test early.
- **`try_from` empty/all-optional rejection** surfaces as serde/`TypeError` strings in a few reconstruction paths — make sure those degrade to a clean diagnostic, not a panic.
- **Bootstrap descriptor** change is the highest-risk single edit (it gates branch load); test it in isolation.
- **Browser query relaxation** (Phase 6) is easy to miss since it's untyped JSON; without it, optional attributes work natively but still over-constrain in the UI.

## Suggested landing order

Phase 0 → 1 (compiles, no behavior change, reviewable on its own) → 5 (storage helpers) → 2 (declare) → 3 (assert) → 4 (query) → 6 (LSP/browser) → 7 (tests/docs), with a focused evaluator optional-match test pulled forward right after Phase 1 to de-risk engine support.
