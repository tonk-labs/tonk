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

## Part B — analyze chain + two sub-phases

### B.1 Public surface

```rust
// tonk-schema/src/analyzer.rs — mirrors evaluate / induce.

pub trait TransactionAnalyzeExt<'a> {
    /// Stage analysis of `syntax` against this transaction.
    fn analyze<'s>(self, syntax: &'s Syntax) -> Analyze<'a, 's>;
}
impl<'a> TransactionAnalyzeExt<'a> for Transaction<'a> {
    fn analyze<'s>(self, syntax: &'s Syntax) -> Analyze<'a, 's> {
        Analyze { txn: self, syntax }
    }
}

pub struct Analyze<'a, 's> { txn: Transaction<'a>, syntax: &'s Syntax }

impl<'a, 's> Analyze<'a, 's> {
    /// Resolve + expand `syntax` against the transaction's
    /// overlay. `env` supplies query capability. The transaction
    /// is the query target, so an in-document concept declared in
    /// an earlier statement is visible to a later one.
    pub async fn perform<Env: QueryEnv>(self, env: &Env)
        -> Result<Analyzed<'a>, AnalyzeError>
    {
        // The schema lookups query through the transaction's
        // overlay — see open item on the txn → Source handle.
        let resolved = resolve(self.syntax, &self.txn, env).await?;
        let expanded = expand(resolved)?;
        Ok(Analyzed { txn: self.txn, analysis: expanded.into_analysis() })
    }
}
```

`Evaluate::perform` is rewired to call `txn.analyze(syntax)`
internally instead of building a `BranchResolver`. Usage:

```rust
branch.transaction().analyze(syntax).perform(env)        // analysis only
branch.transaction().evaluate(syntax).perform(branch, env)   // unchanged
```

### B.2 Sub-phase 1 — resolve + annotate

Walks the syntax tree; resolves every reference through the
`Source` it is handed (via `ResolvedConcept::resolve(...)
.perform(env)` and siblings); attaches descriptors; records
`declarations` / `variables`; emits diagnostics carrying source
spans. **Output keeps source shape** — `concept!`, `rule!`, domain
heads, anchors all still distinct — plus resolution annotations
and a stable per-expression id.

```rust
struct ResolvedDocument {
    exprs: Vec<ResolvedExpr>,           // document order
    diagnostics: Vec<AnalyzeDiagnostic>,// carry source spans
    declarations: HashMap<String, Entity>,
    variables: HashMap<String, Entity>,
}

#[derive(Copy, Clone)]
struct ExprId(usize);                   // stable source back-pointer

struct ResolvedExpr { id: ExprId, label: String, span: Range, kind: ResolvedKind }

enum ResolvedKind {
    Query(ResolvedQuery),
    Assertion(ResolvedAssertion),       // source shape: concept|domain, this, anchor
    Declaration(ResolvedDeclaration),   // concept! / attribute! — stay special for now
    Rule(ResolvedRule),
}

struct ResolvedAssertion {
    head: ResolvedHead,                 // Concept | Domain
    this: ThisIntent,                   // consumed by expand; not in ExpandedDocument
    anchor: Option<String>,             // consumed by expand; not in ExpandedDocument
    fields: Vec<ResolvedField>,
}

enum ResolvedHead {
    Concept { predicate: PredicateDescriptor },  // Durable | Transient
    Domain  { domain: String },                  // descriptor synthesized in expand
}
```

### B.3 Sub-phase 2 — expand

Lowers the annotated tree into **kernel-shaped** forms: every
mutation is a concept-assert. No `Domain`, no `anchor`, no
`ThisIntent`.

```rust
struct ExpandedDocument {
    queries: Vec<ExpandedQuery>,
    mutations: Vec<SourcedMutation>,
    effects: Vec<Effect>,
    declarations: HashMap<String, Entity>,
    variables: HashMap<String, Entity>,
    diagnostics: Vec<AnalyzeDiagnostic>,
}

/// A kernel mutation + its source expression. `Mutation` is the
/// SHARED core reused verbatim from `tonk-schema::mutation` — the
/// same type the /transact wire path uses. The wrapper carries
/// only what downstream still needs: the source expr id, for
/// projecting results back into the user's view.
struct SourcedMutation { mutation: Mutation, source: ExprId }
```

Lowerings (the fixed set):

- **domain head → anonymous concept.** Synthesize a
  `ConceptDescriptor` (one `<domain>/<field>` attribute per field,
  cardinality one, no value-type constraint — the existing
  `From<DomainApplication> for ConceptQuery` logic, run here).
  Always `PredicateDescriptor::Durable`.
- **`&anchor` → paired `Name` assert.** The assert, plus a second
  assert of the built-in `Name` concept publishing `id:<anchor>` →
  the subject entity. Both `SourcedMutation`s share the source
  `ExprId`.
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
- `transact::Statement` enum → replaced by `SourcedMutation`.
- `transact::ThisIntent` survives only inside sub-phase 1→2; never
  reaches `Analysis`.
- `MutationAnalysis::transient` side-set — durability now rides on
  `PredicateDescriptor` inside each `Mutation`.
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
   forms today as `labels`; in the new IR each `SourcedMutation` /
   `ExpandedQuery` carries an `ExprId`, and `ResolvedExpr` holds
   `(id, label, span)`. Grouping by `ExprId` recovers the block
   structure even when one source expr lowers to two mutations
   (the anchor case).
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
3. **Part B** — `TransactionAnalyzeExt` + the two sub-phases.
   `Evaluate::perform` rewired through `txn.analyze(syntax)`.
4. **Part C** — collapse `Application` / `Statement` onto
   `mutation::Mutation` + `SourcedMutation`; delete the listed
   types.
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
- `ExprId` representation — a plain `usize` index into
  `ResolvedDocument::exprs`. Sturdier than today's positional
  `labels` parallelism once one source expr → many forms.
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
