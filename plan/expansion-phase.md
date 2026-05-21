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

- **`tonk-notation`** — pure syntax. Parse → `Syntax`. No branch,
  no `Source`. Unchanged.
- **`tonk-schema`** — schema types: concepts, attributes,
  `mutation.rs`, `effect.rs`, and the resolution surface
  (Part A). No pipeline driving.
- **`tonk-analyzer`** — `analyze` + `expand`. Depends on
  `tonk-notation` and `tonk-schema`. Produces an `Analysis`.
- **`tonk-evaluator`** — `evaluate`: plan + commit. Depends on
  `tonk-analyzer`. The notation pipeline driver. (`evaluate`
  currently lives in `tonk-schema`; it moves here.)

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

### `tonk-introspect`

`tonk-introspect` holds the schema-resolution types —
`ConceptReference` / `AttributeReference`, `ConceptDefinition` /
`AttributeDefinition`, `NamedReference`, `NamedEntity`,
`IntrospectionError`.

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
(an anchored assertion → two mutations) is just the associated
type holding a `Vec`: the mutations nest *under* the one
`Analysis<Assertion>`, so there is no back-pointer and no flat
mutation list — the structure mirrors the document.

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
    mutations: Vec<Mutation>,// filled by sub-phase 2
}
enum Predicate {
    Concept(PredicateDescriptor),  // Durable | Transient
    Domain(String),
}
```

### Sub-phase 2 — expand

Lowers each assertion into kernel-shaped mutations and fills
`AssertionAnalysis::mutations` with the resulting `Vec<Mutation>`
(`Mutation` is `tonk-schema::mutation`'s type — the same one
`/transact` uses). Expansion touches only mutations; a query's
`Analysis<Query>` passes through unchanged.

Lowerings:

- **domain predicate → anonymous concept** — synthesize a
  `ConceptDescriptor` (one `<domain>/<field>` attribute per field,
  cardinality one). Always `PredicateDescriptor::Durable`.
- **`&anchor` → paired `Name` assert** — the assert, plus a second
  assert of the built-in `Name` concept publishing `id:<anchor>` →
  the subject entity. Both land in `AssertionAnalysis::mutations`.
- **omitted `this:` → injected `id:<body-digest>`** — `ThisIntent`
  is consumed: `Uri` → that entity, `Variable` → a var term,
  `Derived` → `id:<digest>`.

Resolution precedes expansion — two passes. Each lowering is
terminal (emits only resolved entities, computed URIs, substituted
terms — no new symbolic references), so expansion output never
needs re-resolution.

## Part C — mutation representation

Every mutation reaching the plan stage is a concept-assert.
`mutation.rs`'s `PredicateDescriptor` / `PredicateApplication` /
`Mutation` (concept-only, `Durable | Transient`) are the single
mutation representation, shared by the `/transact` wire path and
the notation path. Durability rides on `PredicateDescriptor` — no
side-set, no separate transient tracking.

Domain heads and `&anchor` are notation sugar; expansion lowers
them away, so they never reach `Mutation`. The kernel stays
concept-only.

## Open items

- **`Transaction` → `Source`.** Confirm how the
  `dialog_query::Transaction` overlay surfaces as `Source` so
  `evaluate`'s internal analysis resolves through it. Fallback if
  awkward: resolve schema against the underlying `&Branch` and
  keep the in-document `Scope` for same-document declarations.
- **`Analyzable` trait name** — pin against any existing
  convention before implementing.
- **`concept!` / `rule!` expansion.** They stay analyzer-special
  for now; only domain / anchor / derived-`this:` lower. The
  `2026-05-16` macro-system note would lower these too.
- **`rule!:` premises** — if domain premises are allowed, the
  domain→anonymous-concept lowering must also run inside rule
  expansion.
- **`tonk-introspect` crate fate** — once it holds only the
  resolved-schema types, decide whether to fold them into
  `tonk-schema` and drop the crate.
- **Macro fixpoint.** A real macro system needs
  `resolve → expand → …` to iterate (a macro can expand into a new
  symbol). The sub-phase split makes that a loop around the same
  two functions later. Not built now.

## Execution order

1. **Part A** — `ConceptReference` / `AttributeReference` +
   `ConceptDefinition` / `AttributeDefinition` resolution handles,
   `NamedReference`, `EmptyStore`; rewire the language server.
2. **Part B** — carve out `tonk-analyzer`; the `syntax.analyze`
   chain; the `Analysis<T>` IR and the two sub-phases.
3. **Part C** — fold the mutation representation onto
   `mutation.rs`'s types.
4. **Crate move** — extract `evaluate` from `tonk-schema` into
   `tonk-evaluator`; the `syntax.evaluate` chain lives there.
   Rewire `tonk-worker`'s `/evaluate` route.

Each step: `cargo fmt --all`, `clippy --all-targets
--all-features -D warnings`, native + wasm tests.
