# Notation pipeline: expansion phase + resolution redesign

Status: **spec, awaiting sign-off.** Once aligned, executed as
tracked tasks. Supersedes the earlier draft notes.

## Motivation

Two problems, one shape:

1. **`Resolver` / `BranchResolver` are the wrong abstraction.** The
   analyzer's `Resolver` trait is already a deprecated shim over
   `tonk_introspect::BranchIntrospection`. `BranchResolver` is a
   `{ branch, env }` adapter that exists only because
   `BranchIntrospection`'s methods are `&self`-only and cannot
   carry an `env`. The notation pipeline should resolve through
   the same `Transaction` it mutates, not a bespoke adapter.

2. **The analyzer's mutation IR is a hand-rolled half-expanded
   form.** `Application::{Concept, Domain}` + `Statement` +
   `ThisIntent` + `&anchor` name are sugar the analyzer collapses
   ad hoc. The macro-system framing (`@gozala/2026-05-16.md`,
   `2026-05-21.md`) says the substrate's only primitive is
   `assert!`; everything else lowers to it. An explicit
   *expansion* step lowers sugar uniformly so downstream code sees
   one kernel-shaped form.

This spec covers both: a two-sub-phase analyzer (resolve, then
expand) and a `.perform(env)`-style introspection surface that
deletes `BranchResolver`.

Out of scope: a real macro system. Expansion here is a fixed set
of hardcoded lowerings. The design leaves room for the macro
system (see "Interleaving") without building it.

## Part A — resolution surface

### A.1 No trait — chain handles on the resolved types

dialog's query layer is built on **`dialog_query::Source`** —
`query<S: Source>(&self, store: &S)`. `Branch` and the
`Transaction` overlay both provide a `Source`. The capability the
schema lookups need *is* `Source`. `Source` is not `dyn`-safe
(`Source: ArtifactStore + Clone + …`), so everything generic over
it is monomorphized — and there is no usable trait object to lose.

No introspection trait is needed. Resolution becomes **chain
handles** on the resolved types, consistent with `evaluate` /
`induce` / `analyze`: a `resolve` associated function stages the
work and returns a handle; `.perform(env)` runs it. This is
already the established shape in `concept.rs` —
`TransientConcept::is_transient(entity)` returns a builder whose
`resolve` runs against `(branch, env)`. We make every schema
lookup follow it, with `.perform(env)` for consistency.

#### Name resolution and concept resolution are separate steps

The schema lookups today come in two flavors: by published name
(`ConceptByName`) and by entity URI (`ConceptByEntity`). These are
genuinely two operations — and the caller always knows statically
which it has. No `Name | Entity` enum is needed; each gets its own
type and `resolve`.

`ResolvedConcept` / `ResolvedAttribute` resolve from a concrete
`Entity`:

```rust
impl ResolvedConcept {
    pub fn resolve<S: Source>(source: &S, entity: Entity)
        -> ResolveConcept<'_, S>;
}
pub struct ResolveConcept<'a, S> { source: &'a S, entity: Entity }
impl<'a, S: Source> ResolveConcept<'a, S> {
    pub async fn perform<Env: QueryEnv>(self, env: &Env)
        -> Result<Option<ResolvedConcept>, IntrospectionError>
    { /* reconstruct the descriptor from `self.entity` */ }
}

impl ResolvedAttribute {
    pub fn resolve<S: Source>(source: &S, entity: Entity)
        -> ResolveAttribute<'_, S>;
}
// …with the matching `perform(env)`.
```

A named reference is a `NamedReference` newtype with its own
`resolve` — the name→entity step:

```rust
/// A published name — `id:<n>` carries the referent claim.
/// Distinct from the `meta::Name` *concept* schema type; this is
/// just the reference, not the stored claim.
pub struct NamedReference(pub String);

impl NamedReference {
    /// Resolve to the entity the name currently points at —
    /// `(id:<n>, dialog.name/referent, ?e)`.
    pub fn resolve<S: Source>(self, source: &S) -> ResolveName<'_, S>;
}
pub struct ResolveName<'a, S> { source: &'a S, name: NamedReference }
impl<'a, S: Source> ResolveName<'a, S> {
    pub async fn perform<Env: QueryEnv>(self, env: &Env)
        -> Result<Option<Entity>, IntrospectionError>;
}
```

A caller with a name runs both steps; a caller with an entity runs
only the second:

```rust
// have a name:
let Some(entity) = NamedReference("person".into())
    .resolve(source).perform(env).await?
else { /* unknown name */ };
let concept = ResolvedConcept::resolve(source, entity)
    .perform(env).await?;

// already have the entity: just the second line.
```

`ResolvedConcept::resolve` does exactly one thing — descriptor
reconstruction from an entity. `NamedReference::resolve` does
exactly one thing — name lookup. No enum, no `From` juggling, no
"which variant" branch; the caller's static knowledge of what it
holds picks the path. `NamedReference` is also the natural home
for the `lookup_named_entity` logic that is a loose free function
in `concept.rs` today.

The reconstruction body (scattered EAV facts → `ConceptDescriptor`)
is the existing logic, unchanged except the query target
generalizes from `&Branch` to `&S: Source`. A `ResolvedConcept`
`perform` whose body needs the field attributes calls
`ResolvedAttribute::resolve(source, attr_entity).perform(env)`
internally — same composition the `ConceptByEntity` →
`AttributeByEntity` builder chain does today.

`NamedReference` is also a candidate for the analyzer to adopt as
the resolved form of a bare-symbol notation reference. Noted, not
required by this spec; see open items.

Enumerations don't resolve to a single value, so they are their
own handles of the same shape:

```rust
pub fn list_concepts<S: Source>(source: &S) -> ListConcepts<'_, S>;
pub fn list_named_entities<S: Source>(source: &S) -> ListNamedEntities<'_, S>;
// .perform(env) -> Vec<ResolvedConcept> / Vec<NamedEntity>
```

The existing `ConceptLookup` / `AttributeByName` /
`AttributeByEntity` builder structs are replaced by these. The
by-name lookups (`ConceptByName`, `AttributeByName`) become the
caller-side `NamedReference::resolve` step; the by-entity
reconstruction becomes `ResolvedConcept::resolve` /
`ResolvedAttribute::resolve`. Both shed `&Branch` for
`&S: Source` and rename `.resolve(branch, env)` → `.perform(env)`.

**Deleted:**
- `tonk_introspect::BranchIntrospection`.
- `tonk_introspect::SystemIntrospection` /
  `RepositoryIntrospection` — speculative stub traits with zero
  implementors and zero callers anywhere in the workspace.
  Re-introduce against a real shape when a consumer appears.
- `analyzer::resolver::Resolver`, `NoopResolver`.
- `tonk_schema::evaluate::BranchResolver`.
- the `analyzer` `ResolvedConcept` / `ResolvedAttribute` mirrors +
  `From` shims — the `tonk_introspect` structs are the one home.
- the `ConceptLookup` / `AttributeByName` / `AttributeByEntity`
  builders — folded into the `resolve` handles.

With every trait gone, `tonk-introspect` is reduced to the
resolved-schema structs (`ResolvedConcept`, `ResolvedAttribute`,
`NamedEntity`) and `IntrospectionError`. Worth folding those into
`tonk-schema` and dropping the `tonk-introspect` crate entirely —
flagged as an open item.

### A.2 The analyzer's sub-phase 1 takes `&S: Source`

`resolve(syntax, source, env)` — generic over `S: Source`. For a
named reference it runs `NamedReference(name).resolve(source)
.perform(env)` then `ResolvedConcept::resolve(source, entity)
.perform(env)`; for an already-resolved entity it
skips straight to the second step. No trait, no trait object
anywhere in `tonk-schema`. Analyzing against a `Transaction`
overlay (so in-document declarations from earlier statements are
visible) is just passing the transaction instead of a branch.

The document-only path (no branch — the language server's
parse-diagnostics step) passes an **empty `Source`**: a trivial
fact-less store. It lives in `tonk-introspect` (or `tonk-schema`)
as `EmptyStore` and replaces `NoopResolver`. Being a `Source`,
every `resolve(...).perform(env)` against it returns `Ok(None)` /
empty because the store holds no facts.

### A.3 The one `dyn` boundary: the language server

The language server's `IntrospectionFactory::for_uri` is genuinely
late-bound — URI → branch, resolved per request, async — and today
returns `Arc<dyn BranchIntrospection + Send + Sync>` so the
completion/hover helpers can be non-generic.

`Source` is not `dyn`-safe, so the boundary cannot be
`Arc<dyn Source>`. But the LSP does **not** need a query target as
a trait object — it needs the *answers* the lookups produce,
late-bound. The `dyn` boundary becomes a boxed async closure:

```rust
// tonk-language-server — the only dyn boundary; a boxed async fn,
// not a trait object.
type ConceptResolver =
    Arc<dyn Fn(&str) -> BoxFuture<Option<ResolvedConcept>> + Send + Sync>;
```

`IntrospectionFactory::for_uri` returns a small bundle of such
closures (concept lookup, attribute lookup, the two enumerations).
Each closure is built at the factory, where the concrete branch
`Source` *is* known, by capturing it and running the two steps —
`NamedReference(name).resolve(&src).perform(&env)` then
`ResolvedConcept::resolve(&src, entity).perform(&env)` — inside.
The completion/hover helpers take the bundle; `tonk-schema` stays
fully monomorphic.

One boxed-closure boundary at the LSP edge, versus a
`dyn`-dispatched trait threaded through three crates.

> Decision recorded. Iterated through several shapes — a
> `QuerySource` abstraction, an `Introspection` trait
> blanket-implemented over `Source`, plain free functions.
> Settled: no trait at all. Resolution is `resolve` associated
> functions on the resolved types returning chain handles, with
> `.perform(env)` — the same idiom as `evaluate` / `induce` /
> `analyze`. `ResolvedConcept` / `ResolvedAttribute` resolve from
> an `Entity`; name resolution is a separate
> `NamedReference::resolve` step the caller runs first.
> `BranchIntrospection` / `Resolver` / `BranchResolver`
> and the `ConceptLookup` builder family are deleted. The single
> unavoidable `dyn` (the LSP's late-bound branch resolution) is a
> boxed closure, not a trait object.

## Part B — analyze / evaluate chains + two sub-phases

### B.0 Crate home

The analyzer + expansion logic moves into its own crate,
**`tonk-analyzer`**. Today it is a `mod analyzer` tangled inside
`tonk-schema` next to `concept.rs` / `evaluate.rs`. It is *not*
folded into `tonk-notation`: `tonk-notation` is pure syntax (parse
→ `Syntax`, no branch, no `Source`); analysis resolves against a
branch and would drag `dialog_query::Source` + the
schema-reconstruction surface into the lightweight parser crate.
`tonk-analyzer` depends on `tonk-notation` (syntax) and the
schema-resolution pieces; `tonk-schema`'s `evaluate` builds on it.

All trait bounds on new code use `dialog_common::ConditionalSend`
/ `ConditionalSync`, never bare `Send` / `Sync` — so the same
source compiles for native (where they expand to `Send + Sync`)
and `wasm32` (where they vanish).

### B.1 Public surface — `syntax.analyze(source)` / `syntax.evaluate(txn)`

The analyzer **never touches a transaction** — it only reads
(resolution queries) and produces an `Analysis` value. So `analyze`
takes a `Source`, not a `Transaction`. Evaluation *does* produce
mutations, so it takes a `Transaction`. The chains hang off
`Syntax` (the thing being analyzed), not off the source:

```rust
// tonk-analyzer — analysis is pure-read; Source is the input.
pub trait SyntaxAnalyzeExt {
    fn analyze<S: Source>(&self, source: S) -> Analyze<'_, S>;
}
impl SyntaxAnalyzeExt for Syntax { /* … */ }

pub struct Analyze<'s, S> { syntax: &'s Syntax, source: S }
impl<'s, S: Source> Analyze<'s, S> {
    pub async fn perform<Env: QueryEnv + ConditionalSync>(self, env: &Env)
        -> Result<Analysis, AnalyzeError>
    {
        let resolved = resolve(self.syntax, &self.source, env).await?;
        let expanded = expand(resolved)?;
        Ok(expanded.into_analysis())
    }
}
```

```rust
// tonk-schema/src/evaluate.rs — evaluation needs a transaction.
pub trait SyntaxEvaluateExt {
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a>;
}
impl SyntaxEvaluateExt for Syntax { /* … */ }
// Evaluate::perform analyzes against the txn (its overlay is a
// Source, so in-document declarations from earlier statements
// resolve), applies mutations to the txn, returns Evaluated.
```

Usage:

```rust
syntax.analyze(branch).perform(env)                  // analysis only — read
syntax.evaluate(branch.transaction()).perform(env)   // mutate — caller makes the txn
```

A query is read-only, so callers that only run queries go through
`analyze` against the `Branch` directly — no transaction. A
document that mutates goes through `evaluate`, and the caller
creates the `Transaction` and hands it in.

> Open: whether the `Transaction` overlay satisfies `Source` so
> `evaluate`'s internal `analyze` resolves through it. See open
> items — does not change the public shape, only whether
> `evaluate` analyzes against the overlay or the underlying
> branch.

### B.2 Sub-phase 1 — resolve + annotate

Walks the syntax tree; resolves every reference through the
`Source` it is handed (`NamedReference::resolve` →
`ResolvedConcept::resolve`); attaches descriptors; records
`declarations` / `variables`; emits diagnostics carrying source
spans. **Output keeps source shape** — `concept!`, `rule!`, domain
heads, anchors all still distinct — plus resolution annotations.

#### `Analysis<T>` — source-paired by construction

Rather than mirror types (`ResolvedExpression`, `ResolvedQuery`,
…) plus a side `ExpressionId` back-pointer, sub-phase 1's IR pairs
each *parsed* syntax node with its analysis through one generic:

```rust
// tonk-analyzer.
pub trait Analyzable {
    /// The analysis payload computed for this syntax node.
    type Analysis;
}

/// A syntax node paired with its analysis. The source pairing is
/// structural — no `ExpressionId`, no parallel `Vec`. Diagnostics
/// and result projection read the span / label straight off
/// `.source`.
pub struct Analysis<T: Analyzable> {
    pub source: T,
    pub analysis: T::Analysis,
}
```

So the "resolved expression" is `Analysis<Expression>`, a resolved
assertion is `Analysis<Assertion>`, and the whole document is
`Analysis<Syntax>` whose `.analysis` holds
`Vec<Analysis<Expression>>`. The type *is* the back-pointer —
`ExpressionId` is gone.

`tonk_notation` already exposes the per-variant payloads
(`Query`, `Assertion`, `Rule`) under the `Expression` enum, so the
`Analyzable` impls land on those concrete types. The per-node
analysis payloads still have to be written — `Assertion::Analysis`
is a struct — but the *wrapper* types and the back-pointer
plumbing are gone.

```rust
impl Analyzable for Assertion {
    type Analysis = AssertionAnalysis;
}
struct AssertionAnalysis {
    predicate: Predicate,       // Concept | Domain — was `head`
    this: ThisIntent,           // consumed by expand
    anchor: Option<String>,     // consumed by expand
    fields: Vec<FieldAnalysis>,
}
enum Predicate {
    Concept { descriptor: PredicateDescriptor },  // Durable | Transient
    Domain  { domain: String },                   // synthesized in expand
}
```

(`head` is gone — an assertion's predicate is the `predicate`
field, matching `mutation.rs`'s `PredicateDescriptor` vocabulary
Part C consolidates onto.)

#### One-to-many is just the associated type

Expansion can turn one source assertion into several mutations (an
anchored assertion → assert + `Name` assert). This does **not**
break `Analysis<T>`: the multiplicity lives in the associated
type. `Assertion::Analysis` simply *contains* the lowered
mutations — a `Vec` — so the several mutations are **nested under**
the one `Analysis<Assertion>` rather than flattened into a list
that each needs a back-pointer.

So `Analysis<T>` carries all the way through, sub-phase 2
included:

- `Analysis<Syntax>` — `.analysis` holds `Vec<Analysis<Expression>>`.
- `Analysis<Expression>` — `.analysis` dispatches per variant.
- `Analysis<Assertion>` — `.analysis` is the resolved-then-expanded
  payload, which *includes* the `Vec<Mutation>` it lowered to.

The source pairing is structural at every level, and there is no
`ExpressionId` and no per-mutation back-pointer — a lowered
mutation belongs to its source assertion by *containment*.

The one consequence: downstream (the evaluator,
`render_match_blocks`) iterates a *tree* — `Analysis<Syntax>` →
expressions → per-assertion mutations — rather than a flat
`Vec<MutationAnalysis>`. That mirrors the document structure and
is if anything cleaner; the evaluator's current flat
`analysis.mutate.statements` walk becomes a nested walk.

> Open: the exact name of the `Analyzable` trait. Recorded as the
> user's earlier `Analysis<T>` design; pin against any existing
> convention before executing Part B.

### B.3 Sub-phase 2 — expand

Lowers the sub-phase-1 tree into **kernel-shaped** forms: every
mutation is a concept-assert — no `Domain`, no `anchor`, no
`ThisIntent`.

Expansion **only touches mutations.** A query has no anchor, no
derived-`this:` write, no durability — nothing to lower. A query's
`Analysis<Query>` passes through unchanged. (Hence no separate
"expanded query" type — a phantom in the draft sketch; dropped.)

For an assertion, expansion fills in `Assertion::Analysis` — the
resolved-then-lowered payload. Because one assertion can lower to
several mutations, that payload *contains* a `Vec` of kernel
`Mutation`s (reused verbatim from `tonk-schema::mutation` — the
same type `/transact` uses):

```rust
impl Analyzable for Assertion {
    type Analysis = AssertionAnalysis;
}
struct AssertionAnalysis {
    /// The kernel mutations this assertion lowered to. Usually
    /// one; an anchored assertion lowers to two (the assert plus
    /// a `Name` assert). All belong to the enclosing
    /// `Analysis<Assertion>` by containment — no back-pointer.
    mutations: Vec<Mutation>,
}
```

> Naming note. The *current* `transact::MutationAnalysis`
> (`{ statements, requires, transient }` — the whole write-side
> bundle) is **removed** by Part C. Durability is gone from it
> (it rides on `PredicateDescriptor`); the flat `statements` list
> is replaced by the nested `Analysis<Syntax>` tree; `requires`
> either moves onto the top-level analysis or is recomputed. The
> `MutationAnalysis` *name* is not reused — the per-assertion
> payload is `AssertionAnalysis` and the kernel unit is plain
> `Mutation`. No collision; the old bundle simply ceases to exist.

Lowerings (the fixed set):

- **domain predicate → anonymous concept.** Synthesize a
  `ConceptDescriptor` (one `<domain>/<field>` attribute per field,
  cardinality one, no value-type constraint — the existing
  `From<DomainApplication> for ConceptQuery` logic, run here).
  Always `PredicateDescriptor::Durable`.
- **`&anchor` → paired `Name` assert.** The assert, plus a second
  assert of the built-in `Name` concept publishing `id:<anchor>` →
  the subject entity. Both land in the one assertion's
  `AssertionAnalysis::mutations` vec.
- **omitted `this:` → injected `id:<body-digest>`.** `ThisIntent`
  is consumed here: `Uri` → that entity, `Variable` → a var term,
  `Derived` → `id:<digest>`.

### B.4 Interleaving

For this fixed lowering set, **resolution strictly precedes
expansion** — two passes, no interleaving. Sound because each
lowering is *terminal*: it emits only resolved entities, computed
URIs (`id:<anchor>`, `id:<digest>`), and substituted terms — never
a new symbolic reference. So expansion output never needs
re-resolution.

A real macro system would need a `resolve → expand → …` fixpoint
(a macro can expand into a new symbol or a new macro-invocation).
The sub-phase split is what makes that a loop around the same two
functions later, rather than a rewrite. Not built now.

## Part C — consolidation

With expansion in place, every mutation reaching the plan stage is
a concept-assert. The analyzer's mutation IR collapses onto the
shared `mutation.rs` types:

**Deleted:**
- `tonk_introspect::BranchIntrospection` trait (Part A).
- `analyzer::resolver::Resolver`, `NoopResolver`, and the
  `analyzer` `ResolvedConcept` / `ResolvedAttribute` mirrors + `From`
  shims (Part A).
- `tonk_schema::evaluate::BranchResolver` (Part A).
- `transact::Application` enum (`Concept | Domain`).
- `transact::Statement` enum → gone. The write side is the
  `Vec<Mutation>` nested in each `AssertionAnalysis`.
- `transact::ThisIntent` survives only inside sub-phase 1→2; never
  reaches `Analysis`.
- `transact::MutationAnalysis` (the `{ statements, requires,
  transient }` bundle) — gone. Durability rides on
  `PredicateDescriptor`; the flat statement list becomes the
  nested `Analysis<Syntax>` tree.
- `ApplicationPlan::name` + `emit_name_assertion` — anchors are
  ordinary mutations.
- `DomainApplication` + plan-time `From<DomainApplication>` — the
  synth runs in expansion.
- `Planner for Application`'s `Domain` arm.

**Kept / reused:**
- `mutation.rs` — `PredicateDescriptor`, `PredicateApplication`,
  `Mutation`. Concept-only, unchanged. Now used by **both** the
  /transact wire path and the notation path.
- `dialog_query::Source` — dialog's existing query abstraction;
  the `resolve` handles are generic over it.
- `tonk_introspect::ResolvedConcept` / `ResolvedAttribute` /
  `NamedEntity` — the single home for resolved-schema types, now
  with `resolve(...).perform(env)` chain handles.
- `QueryMatchBlock` / `QueryResult` — response shape unchanged.
- Diagnostics + spans — unchanged.

`mutation.rs` does **not** gain a `Domain` variant. Domain heads
are sugar; expansion lowers them away before anything reaches
`Mutation`. (This reverses an earlier idea — confirmed: keep the
kernel minimal.)

## Why source info survives expansion

Downstream needs three things, all kept *beside* the IR, not in
its variant shape:

1. **Projection labels** (`render_match_blocks`) — kept parallel to
   forms today as `labels`; in the new IR every `Analysis<T>`
   carries its `source: T` (the parsed node), which holds the
   span / head name. The anchor case — one assertion, two
   mutations — needs no special handling: the mutations are
   nested under the one `Analysis<Assertion>`, so they project as
   one block by construction.
2. **`declarations` / `variables`** — built in sub-phase 1, copied
   through.
3. **Diagnostics** — produced in sub-phase 1 against source spans;
   expansion never touches them.

## Execution order

1. **Land the transient-fixpoint bug fix** (done — notation
   transient assertions seed the effects fixpoint;
   `it_declares_transient_concept_via_notation` passes). Plus the
   Phase-4 snapshot-join fix (done — see #66). Both independent of
   this spec.
2. **Part A** — `resolve(...).perform(env)` chain handles on
   `ResolvedConcept` / `ResolvedAttribute` (+ `list_*` handles),
   generic over `S: Source`; fold the `ConceptLookup` builder
   family into them; `EmptyStore`; delete `BranchIntrospection` /
   `Resolver` / `NoopResolver` / `BranchResolver` and the
   `analyzer` resolved-type mirrors. Language server +
   `lsp_introspection` rewired to the boxed-closure boundary.
3. **Part B** — carve out the `tonk-analyzer` crate; the
   `syntax.analyze(source)` / `syntax.evaluate(txn)` chains; the
   two sub-phases. `ConditionalSend` / `ConditionalSync` on all
   new bounds.
4. **Part C** — collapse `Application` / `Statement` onto
   `mutation::Mutation` nested in the `Analysis<T>` tree; delete
   the listed types.
5. Each step: `cargo fmt --all`, `clippy --all-targets
   --all-features -D warnings`, native + wasm tests.

## Open items

- **`Transaction` → `Source` handle.** The schema lookups are
  generic over `S: Source`. Need to confirm how a
  `dialog_query::Transaction`'s overlay is exposed as something
  satisfying `Source` (the txn itself, a `.source()` accessor, or
  the lookups taking the txn and calling `.query()` internally).
  This is a dialog-API detail to verify before executing Part A;
  it does not change the design, only the call shape. If the
  overlay is awkward to surface as `Source`, fallback: the
  lookups take a `&Branch` for the schema (schema rarely changes
  mid-document) and the analyzer's in-doc `Scope` continues to
  cover same-document declarations — i.e. keep today's split.
- `Analyzable` / `Analysis<T>` ergonomics — confirm the nested
  tree (`Analysis<Syntax>` → `Vec<Analysis<Expression>>` → …) is
  comfortable for the evaluator and `render_match_blocks` to
  walk, versus the flat statement list they use today.
- Whether `concept!` / `rule!` lower through expansion too (the
  2026-05-16 ambition) or stay analyzer-special. This spec keeps
  them special; only domain / anchor / derived-`this:` lower.
- `rule!:` premises: if domain premises are allowed, the
  domain→anonymous-concept lowering must also run inside rule
  expansion.
- **Fate of the `tonk-introspect` crate.** Once every trait is
  deleted it holds only `ResolvedConcept` / `ResolvedAttribute` /
  `NamedEntity` / `IntrospectionError` (and now `NamedReference`).
  Decide whether to fold those into `tonk-schema` and remove the
  crate, or keep it as the resolved-types home. Removing it is
  tidier but touches every crate that depends on it — confirm the
  dependency fan-out first.
- **`NamedReference` in the analyzer.** It is the resolved form of
  a bare-symbol notation reference. The analyzer's bare-symbol
  field-value resolution could produce `NamedReference` directly,
  unifying naming across the resolvers and the analyzer. Worth
  doing once Part A lands; not required by the resolution refactor
  itself.
- `EmptyStore` (empty `Source` for the document-only path) — a
  trivial fact-less store. Confirm where it lives so both the
  analyzer's no-branch path and the language server's
  parse-diagnostics step share it.
