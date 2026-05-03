# `analyze`: spec by example

`analyze(syntax, branch_resolver) -> Result<Analysis, Diagnostic>`
walks a parsed asserted-notation document and produces an
`Analysis` — the value the worker uses to (a) execute queries,
(b) commit transactions, and (c) shape the response.

This document defines `Analysis` by working through the
expression shapes the parser supports.

---

## Shape

`Analysis` is one struct, not an enum. A document may contain
queries, mutations, or both. The shape supports all three
without dispatch; query-only docs leave `mutate.statements`
empty, mutation-only docs leave `query` as `None`.

### Three phases of analysis

Producing an `Analysis` runs three phases:

1. **Derive.** Walk every head in the document and compute
   what we can without the branch:
   - Bookmark-form heads (`attribute! foo:`, `concept! foo:`,
     `person! alice:`) get a derived entity registered in
     `declarations` keyed by the bookmark name.
   - Variable-form heads (`attribute! ?foo:`, `concept! ?foo:`,
     `person! ?alice:`) get a derived entity registered in
     `variables` keyed by the variable name.
   - Anonymous heads register nothing — their entity will
     live directly inside the relevant `Application`.

2. **Build the query.** For each query expression, build an
   `Application`. Substitute `.bookmark` references against
   `declarations` (and the branch resolver as fallback) into
   `Term::Constant(<entity>)`. Substitute `?var` references
   against `variables` into `Term::Constant(<entity>)`. Store
   the per-expression `Application`s on `QueryAnalysis` in
   source order. The unified `ConceptApplication` the engine
   evaluates is derived on demand via
   `ConceptApplication::from(&query_analysis)` — combining
   every expression's predicate into one.

3. **Build the mutation plan.** For each mutation expression,
   build an `Application`. Substitute `.bookmark` references
   the same way as in Phase 2 (analysis-time-substituted). Do
   *not* substitute `?var` — those stay as
   `Term::Variable(name)` until planning time, because some
   may be query-bound. Store as `Statement::Assert(_)` or
   `Statement::Retract(_)` in document order.

### Resolution rules summarized

| Reference         | When substituted   | Source                          |
|-------------------|--------------------|---------------------------------|
| `.bookmark`       | analysis (Phase 2/3) | `declarations`, fallback to branch resolver |
| `?var` in query   | analysis (Phase 2) | `variables` only                |
| `?var` in mutation | planning time      | `variables ∪ query_bindings`    |
| URI               | parsed inline      | n/a                             |

When `?var` is found in `variables`, it's substituted to a
`Term::Constant` immediately. When it's not found, it stays
as `Term::Variable(name)` and becomes either a query binding
(if found in queries) or an analyzer error (if no source
binds it).

The analyzer enforces that `declarations.keys()` and
`variables.keys()` are disjoint (no name shadowing across the
two), and that `mutate.requires ⊆ query.bindings()` (every
unbound `?var` in a mutation is bound by some query).

---

## Analysis types

```rust
pub struct Analysis {
    /// `.foo` → entity. Bookmark-form heads
    /// (`attribute! foo:`, `concept! foo:`, `person! alice:`).
    /// Substituted at analysis time into both queries and
    /// mutations; kept here for the editor's "you defined
    /// these names" introspection view.
    pub declarations: HashMap<String, Entity>,

    /// `?foo` → entity. Variable-form heads (`attribute! ?foo:`
    /// etc.) where the entity is content-derived. Used as
    /// parameter substitutions when building the unified
    /// query (Phase 2), and merged with query-bound values
    /// when planning mutations (Phase 3).
    pub variables: HashMap<String, Entity>,

    /// Read side. `None` for pure-mutation documents.
    pub query: Option<QueryAnalysis>,

    /// Write side. `mutate.statements` is empty for pure-
    /// query documents.
    pub mutate: MutationAnalysis,
}

// ---------------------------- read side --------------------------

pub struct QueryAnalysis {
    /// Per-source-expression `Application`s, in document
    /// order, with `declarations` and `variables` already
    /// substituted in. The renderer uses these to project
    /// each match back into the user's view ("for the
    /// `person ?alice:` expression, here are the matches").
    pub queries: Vec<Application>,
}

impl QueryAnalysis {
    /// Names of the `Term::Variable` slots that survived
    /// `variables` substitution — i.e., what this query
    /// binds at evaluation time.
    pub fn bindings(&self) -> Vec<String> { ... }
}

impl From<&QueryAnalysis> for ConceptApplication {
    /// Combine every `queries[i]`'s predicate into one
    /// unified `ConceptApplication` whose terms union the
    /// per-expression terms. Shared variable names join the
    /// expressions (a `?alice` in two queries means matches
    /// must agree on `alice`). The engine evaluates the
    /// returned application once per request.
    fn from(query: &QueryAnalysis) -> Self { ... }
}

// --------------------------- write side --------------------------

pub struct MutationAnalysis {
    /// In document order. Each `Application` has had
    /// `.bookmark` references substituted to constants but
    /// keeps `?var` references as variables — substitution
    /// happens at planning time.
    pub statements: Vec<Statement>,

    /// Variable names this plan reads from query bindings.
    /// Disjoint from `variables.keys()` (the analyzer
    /// enforces). Subset of `query.bindings()` (the analyzer
    /// also enforces).
    pub requires: HashSet<String>,
}

impl MutationAnalysis {
    /// Find a named-attribute statement. Scans `statements`
    /// for an `Assert(Application::Concept(_))` whose
    /// predicate is the built-in `attribute` schema and
    /// whose `name` parameter matches.
    pub fn attribute(&self, name: &str) -> Option<&Application> { ... }
    /// Same for named-concept statements.
    pub fn concept(&self, name: &str) -> Option<&Application> { ... }
    /// Iterate all attribute statements (no name filter).
    pub fn attributes(&self) -> impl Iterator<Item = &Application> { ... }
    /// Iterate all concept statements.
    pub fn concepts(&self) -> impl Iterator<Item = &Application> { ... }
}

pub enum Statement {
    Assert(Application),
    Retract(Application),
}

// -------------- shared between read and write sides --------------

/// Same applied-form types used for queries — predicate plus
/// parameters. The mutation side and query side share these
/// because both express "a predicate applied to specific
/// terms," and the only difference is whether you want to
/// find facts matching the pattern (query) or commit /
/// dissociate them (mutation).
pub enum Application {
    /// `person …:` head — `ConceptApplication` is dialog's
    /// `ConceptQuery` (`{ predicate: ConceptDescriptor, terms: Parameters }`),
    /// produced by resolving the concept against the branch
    /// (or in-document state) and applying the user's terms.
    Concept(ConceptApplication),
    /// `xyz.tonk …:` head — synthesized inline because claim
    /// domains have no schema to look up.
    Domain(DomainApplication),
}

impl Application {
    pub fn parameters(&self) -> &Parameters { ... }
    pub fn bindings(&self) -> Vec<String> { ... }
}

pub type ConceptApplication = ConceptQuery;

pub struct DomainApplication {
    pub domain: String,
    pub parameters: Parameters,
}

impl From<DomainApplication> for ConceptApplication {
    /// Synthesize a `ConceptDescriptor` with attribute
    /// `<domain>/<key>` per parameter (no value-type
    /// constraint), then `descriptor.apply(parameters)`.
    fn from(d: DomainApplication) -> Self { ... }
}

// -------------------------- planning -----------------------------

pub trait Planner {
    type Output;
    /// Substitute `Term::Variable(name)` slots in `self`'s
    /// parameters using `bindings[name]`, returning a fully
    /// concrete `Output`. Errors when a variable is unbound.
    fn plan(self, bindings: &Parameters) -> Result<Self::Output, PlanError>;
}

impl Planner for Application {
    type Output = ApplicationPlan;
    fn plan(self, bindings: &Parameters) -> Result<ApplicationPlan, PlanError> {
        // 1. Coerce Domain to Concept via DomainApplication's
        //    `From<DomainApplication> for ConceptQuery`.
        // 2. Substitute every Term::Variable in `terms` with
        //    `bindings[name]`. Error if any name is missing.
        // 3. Wrap the substituted ConceptQuery in ApplicationPlan.
    }
}

/// Fully concrete, ready to commit. Wraps a substituted
/// [`ConceptQuery`]; `assert` walks the predicate's `with` map
/// and emits one EAV per non-blank field; `retract` mirrors via
/// dissociate.
///
/// The same shape carries every concept — including the
/// built-in `attribute` and `concept` schemas, whose fields are
/// real EAV attributes (`dialog.attribute/id` etc.) just like
/// any user concept's. There is no special dispatch arm for
/// built-ins; the bootstrap step writes their descriptors to
/// `main` + `meta` at repo creation, and from then on they're
/// resolved like any other concept.
pub struct ApplicationPlan {
    pub statement: ConceptQuery,
}

impl Statement for ApplicationPlan {
    fn assert(self, update: &mut impl Update) {
        // emit one (the, of=this, value) per field where the
        // term is Term::Constant; skip blanks/variables.
    }
    fn retract(self, update: &mut impl Update) {
        // mirror with dissociate.
    }
}
```

### Evaluation flow

```
let analysis = analyze(syntax, &branch_resolver).await?;

// Build the base bindings frame from analysis-derived
// variables. Same for every query match.
let mut base = Parameters::new();
for (name, entity) in &analysis.variables {
    base.insert(name.clone(), Term::from(entity.clone()).into());
}

// Run the query (or use a single empty frame if none).
let matches: Vec<Parameters> = match &analysis.query {
    Some(q) => ConceptApplication::from(q).evaluate(env).await?,
    None => vec![Parameters::new()],
};

// Plan and commit per binding frame.
for match_frame in matches {
    let mut frame = base.clone();
    frame.extend(match_frame);
    for stmt in &analysis.mutate.statements {
        let app = stmt.application().clone();
        let plan = app.plan(frame.clone())?;
        match stmt {
            Statement::Assert(_)  => tx.assert(plan),
            Statement::Retract(_) => tx.retract(plan),
        }
    }
}
```

---

## Example 1: define one attribute

**Input:**

```yaml
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: The person's name
```

**Analysis:**

```rust
Analysis {
    declarations: { "person-name": the:Hb… },   // Entity::of("person-name")
                                                // OR descriptor.to_uri();
                                                // analyzer picks one canonically.
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Assert(Application::Concept(ConceptApplication {
                predicate: <built-in `attribute` schema>,
                terms: {
                    "this":        Term::Constant(the:Hb…),
                    "id":          Term::Constant("io.gozala.person/name"),
                    "type":        Term::Constant("Text"),
                    "cardinality": Term::Constant("one"),
                    "description": Term::Constant("The person's name"),
                    "name":        Term::Constant("person-name"),
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

At evaluation time, `Planner::plan` sees the predicate is the
built-in `attribute` schema and the `name` parameter is set,
so it produces `ApplicationPlan::NamedAttribute(NamedAttribute { … })`.
That value impls `Statement`; `tx.assert(plan)` writes the
five claims (`id`, `type`, `cardinality`, `description`, `name`).

---

## Example 2: define attribute, then concept, in one document

**Input:**

```yaml
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one

attribute! person-age:
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one

concept! person:
  with:
    name: .person-name
    age:  .person-age
```

**Analysis:**

```rust
Analysis {
    declarations: {
        "person-name": the:Hb…,
        "person-age":  the:Wx…,
        "person":      concept:Pq…,
    },
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            // Two attribute statements (terms elided for brevity).
            Statement::Assert(Application::Concept(/* attribute! person-name */)),
            Statement::Assert(Application::Concept(/* attribute! person-age */)),
            // Concept statement: `.person-name` / `.person-age`
            // were already substituted to entity URIs from
            // `declarations`.
            Statement::Assert(Application::Concept(ConceptApplication {
                predicate: <built-in `concept` schema>,
                terms: {
                    "this":        Term::Constant(concept:Pq…),
                    "with.name":  Term::Constant(the:Hb…),  // from declarations
                    "with.age":   Term::Constant(the:Wx…),  // from declarations
                    "name":        Term::Constant("person"),
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

The "with-map" in `concept!` body materializes as multiple
parameters (one per field) on the concept-schema predicate.
The exact key shape (`with.name` shown above) is a detail of
how `MutationAnalysis::attribute()` / `concept()` accessors
read these back; treat it as illustrative, the canonical
shape is whatever the built-in `concept` schema declares.

---

## Example 3: same as above with variables instead of bookmarks

**Input:**

```yaml
attribute! ?person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one

attribute! ?person-age:
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one

concept! person:
  with:
    name: ?person-name
    age:  ?person-age
```

**Analysis:**

```rust
Analysis {
    declarations: { "person": concept:Pq… },
    variables: {
        "person-name": the:Hb…,
        "person-age":  the:Wx…,
    },
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Assert(Application::Concept(/* attribute! ?person-name —
                same shape as Example 1 minus the `name` parameter */)),
            Statement::Assert(Application::Concept(/* attribute! ?person-age */)),
            // The concept's `with` references `?person-name` /
            // `?person-age`. These are NOT substituted at
            // analysis time — they stay as Term::Variable.
            // Planning substitutes from `variables`.
            Statement::Assert(Application::Concept(ConceptApplication {
                predicate: <built-in `concept` schema>,
                terms: {
                    "this":        Term::Constant(concept:Pq…),
                    "with.name":  Term::Variable("person-name"),
                    "with.age":   Term::Variable("person-age"),
                    "name":        Term::Constant("person"),
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

The two attributes are reachable by name within this document
via `?person-name` / `?person-age`, but no `dialog.meta/name`
claim is written. After commit, future documents can't
resolve `.person-name` against the branch — that name was
doc-scope only.

---

## Example 4: assert an instance referencing in-doc bookmark

**Input** (assuming `person-name`, `person-age`, and the
`person` concept were defined in a prior commit):

```yaml
person! alice:
  name: "Alice"
  age:  28
```

**Analysis:**

```rust
Analysis {
    declarations: { "alice": did:key:zAlice… },   // Entity::of("alice")
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Assert(Application::Concept(ConceptApplication {
                predicate: <Person concept resolved from branch>,
                terms: {
                    "this": Term::Constant(did:key:zAlice…),
                    "name": Term::Constant("Alice"),
                    "age":  Term::Constant(28u64),
                    // Bookmark binding gets a meta/name claim
                    // alongside the user's fields. The exact
                    // representation is up to the planner.
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

`person`'s descriptor comes from the branch via
`branch_resolver.resolve_concept("person")`. The planner will
emit `ApplicationPlan::PredicateApplication`, which wraps a
`ConceptStatement` ready for `tx.assert`.

---

## Example 5: retract a concept-projection

**Input:**

```yaml
person! did:key:zAlice…: _
```

**Analysis:**

```rust
Analysis {
    declarations: {},
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Retract(Application::Concept(ConceptApplication {
                predicate: <Person concept from branch>,
                terms: {
                    "this": Term::Constant(did:key:zAlice…),
                    "name": Term::blank(),
                    "age":  Term::blank(),
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

The body's `_` becomes `Term::blank()` for every field of the
concept's `with` schema. The `Retract` plan first runs the
application as a query — bound terms (`this` here) anchor
the match, blank terms accept any value — and dissociates
exactly the facts found. The engine's existing query-then-
dissociate machinery handles this; we don't need a separate
retraction shape on the analyzer side.

### Partial retraction (selective field)

```yaml
person!:
  name: "Alice"
  age:  _
```

```rust
Analysis {
    declarations: {},
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Retract(Application::Concept(ConceptApplication {
                predicate: <Person concept from branch>,
                terms: {
                    "this": Term::Variable("__anon_0"),  // anonymous head
                    "name": Term::Constant("Alice"),     // anchors the match
                    "age":  Term::blank(),               // the field to retract
                },
            })),
        ],
        requires: HashSet::new(),
    },
}
```

Important: only the `age` of persons whose `name` is `"Alice"`
gets retracted. The `Term::Constant("Alice")` constrains the
query — no other matches survive — so the dissociation set is
exactly `{(age, this, *) | name(this) == "Alice"}`. Other
attributes on those entities (and `name` itself) are left
intact.

A field is dissociated when its term is `blank` *or*
`Variable`; bound `Constant` fields are pure match anchors
and are not retracted.

---

## Example 6: define schema and an instance, all in one doc

**Input:**

```yaml
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one

concept! person:
  with:
    name: .person-name

person! alice:
  name: "Alice"
```

**Analysis:**

```rust
Analysis {
    declarations: {
        "person-name": the:Hb…,
        "person":      concept:Pq…,
        "alice":       did:key:zAlice…,
    },
    variables: {},
    query: None,
    mutate: MutationAnalysis {
        statements: [
            Statement::Assert(Application::Concept(/* attribute! person-name */)),
            Statement::Assert(Application::Concept(/* concept! person —
                with.name: Term::Constant(the:Hb…) substituted from declarations */)),
            Statement::Assert(Application::Concept(/* person! alice —
                this: Term::Constant(did:key:zAlice…), name: "Alice" */)),
        ],
        requires: HashSet::new(),
    },
}
```

The third statement (`person! alice:`) needs the `person`
concept's descriptor to type-check `name` and produce the
right predicate. The analyzer looks `person` up in
`declarations` first (found, set during Phase 1), so it
doesn't need to consult the branch.

---

## Example 7: query then assert, joined by a variable

**Input:**

```yaml
person ?alice:
  name: "Alice"

person! ?alice:
  current: true
```

**Analysis:**

```rust
Analysis {
    declarations: {},
    variables: {},                                // ?alice is query-bound
    query: Some(QueryAnalysis {
        queries: [
            Application::Concept(/* person ?alice: { name: "Alice" } */),
        ],
    }),
    // ConceptApplication::from(&query) yields a unified app
    // binding ?alice to entities matching `person.name == "Alice"`.
    mutate: MutationAnalysis {
        statements: [
            Statement::Assert(Application::Concept(ConceptApplication {
                predicate: <Person concept from branch>,
                terms: {
                    "this":    Term::Variable("alice"),       // unbound at analysis
                    "current": Term::Constant(true),
                },
            })),
        ],
        requires: HashSet::from(["alice".to_owned()]),  // declares the contract
    },
}
```

`?alice` doesn't appear in `variables` or `declarations` —
it's bound by the query. The analyzer:

- Records `?alice` as a query binding (`query.bindings()` returns `["alice"]`).
- Records `?alice` as a mutation requirement (`mutate.requires == ["alice"]`).
- Verifies `requires ⊆ bindings()` at construction. Holds.

At evaluation time, the engine produces one match per Person
named "Alice". For each, the planner builds the assertion
with `Term::Variable("alice")` substituted to that match's
entity, and the worker commits one assertion per Alice.

