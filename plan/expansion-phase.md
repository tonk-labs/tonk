# Notation pipeline: analyzer + expansion

Status: spec. Describes the target shape of the notation
pipeline: how a parsed document resolves against a branch, lowers
its sugar, and becomes a runnable plan.

The pipeline:

```
parse → resolve → expand → plan → commit
        └──── analyze ────┘
```

`parse` is `tonk-notation` (pure syntax). `resolve` + `expand`
are the analyzer. `plan` + `commit` are evaluation.

## Crate layout

Five crates, each with one charter:

- **`tonk-notation`** — pure syntax. Parse → `Syntax`. No branch,
  no `Source`.
- **`tonk-schema`** — the concepts the evaluation runtime is
  seeded with: concept / attribute / rule definitions, the
  built-in registry, the meta-branch concepts (replica, branch,
  remote, tracking branch), and the resolution surface (Part A)
  that reconstructs a definition from a branch. Everything here
  *is* a schema definition or resolves one.
- **`tonk-core`** — the operations performed against a branch:
  `Claim` (a typed write — Part C), `Query` (a read request),
  `Conclusion` (a read result), `TransactRequest` (a `Claim`
  batch). These are not concepts — they are reads and writes
  *over* concepts. `tonk-core` sits below everything that issues
  an operation: the worker, the UI crates (`tonk-display` /
  `tonk-concept` build a `Query` and render `Conclusion`s), and
  `tonk-evaluator`.
- **`tonk-analyzer`** — `analyze` + `expand`: notation → an
  `Analysis` whose write side is `Claim`s. Depends on
  `tonk-notation`, `tonk-schema`, `tonk-core`.
- **`tonk-evaluator`** — `evaluate`: plan + commit. Runs the
  operations a document resolves to. Depends on `tonk-analyzer`.

The split that matters: `tonk-schema` holds *definitions*
(what shapes exist), `tonk-core` holds *operations* (what you do
against them), `tonk-analyzer` *derives* operations from notation,
`tonk-evaluator` *runs* them.

All trait bounds use `dialog_common::ConditionalSend` /
`ConditionalSync`, so the same source compiles native and
`wasm32`.

## Part A — resolution

Schema lookups query through **`dialog_query::Source`**, which
both `Branch` and the `Transaction` overlay provide. Resolution is
expressed as chain handles — a `resolve` method stages the work,
`.perform(env)` runs it — matching the existing `evaluate` /
`induce` idiom.

A **reference** names a thing; resolving it yields the thing's
**definition**:

```
ConceptReference   --resolve.perform-->  ConceptDefinition
AttributeReference --resolve.perform-->  AttributeDefinition
```

### References

A `ConceptReference` names a concept — by entity, or by published
name. It is constructed via `From`, so the caller never matches a
variant:

```rust
/// Names a concept: a direct entity, or a published name to look
/// up. The name/entity split is internal.
pub struct ConceptReference(/* private: Entity | name */);

impl From<Entity>         for ConceptReference {}
impl From<NamedReference> for ConceptReference {}

impl ConceptReference {
    pub fn resolve<S: Source>(self, source: &S) -> ResolveConcept<'_, S>;
}
pub struct ResolveConcept<'a, S> { source: &'a S, reference: ConceptReference }
impl<'a, S: Source> ResolveConcept<'a, S> {
    pub async fn perform<Env: QueryEnv + ConditionalSync>(self, env: &Env)
        -> Result<Option<ConceptDefinition>, IntrospectionError>;
}
```

`AttributeReference` is the same shape, yielding
`AttributeDefinition`. Per-kind types keep resolution
type-correct — an attribute entity can't be resolved as a concept.

```rust
let definition = ConceptReference::from(entity)
    .resolve(source).perform(env).await?;
// or, starting from a name:
let definition = ConceptReference::from(NamedReference("person".into()))
    .resolve(source).perform(env).await?;
```

`NamedReference` is the published-name newtype — `id:<n>` carries
a `dialog.name/referent` claim. It is the home for the
name→entity lookup, and a `From` source for the typed references.

```rust
/// A published name. Distinct from the `meta::Name` concept
/// schema type — this is the reference, not the stored claim.
pub struct NamedReference(pub String);
```

### Definitions

`ConceptDefinition` / `AttributeDefinition` are the resolved
result — entity + reconstructed descriptor (a concept also carries
its transient flag). `resolve`'s `perform` reconstructs the
descriptor from the entity's EAV facts; a concept's `perform`
resolves each field attribute via
`AttributeReference::from(attr_entity).resolve(source).perform(env)`.

### Enumeration

`list_concepts` / `list_named_entities` are handles of the same
shape:

```rust
pub fn list_concepts<S: Source>(source: &S) -> ListConcepts<'_, S>;
pub fn list_named_entities<S: Source>(source: &S) -> ListNamedEntities<'_, S>;
// .perform(env) -> Vec<ConceptDefinition> / Vec<NamedEntity>
```

### Document-only resolution

The language server's parse-diagnostics path has no branch. It
resolves against `EmptyStore` — a fact-less `Source` — so every
`perform` returns `Ok(None)` / empty.

### Language-server boundary

`IntrospectionFactory::for_uri` is late-bound (URI → branch, per
request). It returns a bundle of boxed async closures — one per
lookup — each capturing the request's concrete `Source`:

```rust
type ConceptLookup =
    Arc<dyn Fn(&str) -> BoxFuture<Option<ConceptDefinition>> + Send + Sync>;
```

This is the one `dyn` boundary; the rest of the pipeline stays
fully monomorphic.

### Where the resolution types live

The resolution surface — `ConceptReference` / `AttributeReference`,
`ConceptDefinition` / `AttributeDefinition`, `NamedReference`,
`NamedEntity`, `IntrospectionError` — lives in `tonk-schema`. A
definition *is* schema; a reference resolves one; resolution
reconstructs one from a branch. All of it is the schema layer.

## Part B — analyze / evaluate chains

The chains hang off `Syntax`:

```rust
syntax.analyze(branch).perform(env)                  // -> Analysis
syntax.evaluate(branch.transaction()).perform(env)   // -> Evaluated
```

`analyze` is pure-read — it takes a `Source` (a `Branch` for the
common case) and produces an `Analysis`. `evaluate` produces
mutations, so the caller creates a `Transaction` and hands it in;
`evaluate` analyzes against that transaction's overlay (so
in-document declarations from earlier statements resolve), applies
mutations to it, and returns `Evaluated`.

```rust
// tonk-analyzer
pub trait SyntaxAnalyzeExt {
    fn analyze<S: Source>(&self, source: S) -> Analyze<'_, S>;
}
pub struct Analyze<'s, S> { syntax: &'s Syntax, source: S }
impl<'s, S: Source> Analyze<'s, S> {
    pub async fn perform<Env: QueryEnv + ConditionalSync>(self, env: &Env)
        -> Result<Analysis, AnalyzeError>;
}

// tonk-evaluator
pub trait SyntaxEvaluateExt {
    fn evaluate<'a>(&self, txn: Transaction<'a>) -> Evaluate<'_, 'a>;
}
```

### The `Analysis<T>` IR

Analysis pairs each parsed syntax node with its computed analysis
through one generic:

```rust
pub trait Analyzable {
    /// The analysis payload computed for this syntax node.
    type Analysis;
}

/// A syntax node paired with its analysis. The source pairing is
/// structural — diagnostics and result projection read the
/// span / head name straight off `.source`.
pub struct Analysis<T: Analyzable> {
    pub source: T,
    pub analysis: T::Analysis,
}
```

So the document is `Analysis<Syntax>`, an expression is
`Analysis<Expression>`, an assertion is `Analysis<Assertion>`. The
`Analyzable` impls land on `tonk_notation`'s concrete payload
types (`Query`, `Assertion`, `Rule`).

The tree:

- `Analysis<Syntax>` — `.analysis` holds `Vec<Analysis<Expression>>`.
- `Analysis<Expression>` — `.analysis` dispatches per variant.
- `Analysis<Assertion>` — `.analysis` is `AssertionAnalysis`.

`Analysis<T>` carries through both sub-phases. One-to-many lowering
(an anchored assertion → two claims) is just the associated type
holding a `Vec`: the claims nest *under* the one
`Analysis<Assertion>`, so there is no back-pointer and no flat
claim list — the structure mirrors the document.

### Sub-phase 1 — resolve + annotate

Walks the syntax tree; resolves every reference through the
`Source` (`ConceptReference::resolve` etc.); attaches
descriptors; records `declarations` / `variables`; emits
diagnostics with source spans. Output keeps source shape —
`concept!`, `rule!`, domain heads, anchors all still distinct —
plus the resolution annotations.

```rust
impl Analyzable for Assertion {
    type Analysis = AssertionAnalysis;
}
struct AssertionAnalysis {
    predicate: Predicate,    // an assertion's predicate
    this: ThisIntent,        // consumed by sub-phase 2
    anchor: Option<String>,  // consumed by sub-phase 2
    fields: Vec<FieldAnalysis>,
    claims: Vec<Claim>,      // filled by sub-phase 2
}
enum Predicate {
    Concept(PredicateDescriptor),  // Durable | Transient
    Domain(String),
}
```

### Sub-phase 2 — expand

Lowers each assertion into kernel-shaped claims and fills
`AssertionAnalysis::claims` with the resulting `Vec<Claim>`
(`Claim` is the typed assert/retract — see Part C, the same one
`/transact` uses). Expansion touches only assertions; a query's
`Analysis<Query>` passes through unchanged.

Lowerings:

- **domain predicate → anonymous concept** — synthesize a
  `ConceptDescriptor` (one `<domain>/<field>` attribute per field,
  cardinality one). Always `PredicateDescriptor::Durable`.
- **`&anchor` → paired `Name` assert** — the assert, plus a second
  assert of the built-in `Name` concept publishing `id:<anchor>` →
  the subject entity. Both land in `AssertionAnalysis::claims`.
- **omitted `this:` → injected `id:<body-digest>`** — `ThisIntent`
  is consumed: `Uri` → that entity, `Variable` → a var term,
  `Derived` → `id:<digest>`.

Resolution precedes expansion — two passes. Each lowering is
terminal (emits only resolved entities, computed URIs, substituted
terms — no new symbolic references), so expansion output never
needs re-resolution.

## Part C — the `Claim` representation

A **`Claim`** is the typed assert/retract of a concept
application — `Assert | Retract` over a `PredicateApplication`
(`PredicateDescriptor` + terms; `Durable | Transient`). It is the
single representation for a fact-write, shared by the `/transact`
wire path and the notation path. Durability rides on
`PredicateDescriptor` — no side-set, no separate transient
tracking. Every claim reaching the plan stage is concept-shaped;
domain heads and `&anchor` are notation sugar that expansion
lowers away first.

`Claim` lives in `tonk-core` (with `PredicateApplication` —
`PredicateDescriptor` + terms — and `TransactRequest`, a `Claim`
batch). It is an operation, not a concept, so it is not in
`tonk-schema`; it carries a `PredicateDescriptor`, so `tonk-core`
depends on `tonk-schema`.

`Claim` is distinct from `dialog_query::Fact` — the raw
`(the, of, is)` EAV triple dialog deals in. `Claim` is the typed,
concept-shaped write; `Fact` is the untyped triple it ultimately
emits. The two names never collide.

## Open items

- **`Transaction` as `Source`.** `evaluate` analyzes against the
  transaction's overlay so a concept declared earlier in the same
  document resolves for a later statement. This requires the
  `Transaction` overlay to satisfy `Source`. If it cannot, schema
  resolution falls back to the underlying `&Branch` and an
  in-document `Scope` covers same-document declarations.
- **`concept!` / `rule!` expansion.** This spec lowers only domain
  predicates, `&anchor`, and derived-`this:`. `concept!` and
  `rule!` are handled directly by the analyzer. A macro system
  (`2026-05-16` note) would lower these the same way.
- **`rule!:` premises.** If a `rule!:` premise may name a domain
  predicate, the domain→anonymous-concept lowering runs inside
  rule expansion as well.
- **Macro fixpoint.** A macro system needs `resolve → expand → …`
  to iterate — a macro can expand into a new symbol that itself
  needs resolution. The two-sub-phase split makes that a loop
  around `resolve` and `expand`. This spec's lowerings are
  terminal, so a single pass suffices.
