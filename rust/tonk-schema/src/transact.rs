//! Asserted-notation analysis output and the planner that turns it
//! into committable statements.
//!
//! See `analysis-spec.md` (sibling to this crate) for the full
//! design. Quick orientation:
//!
//! - [`Analysis`] is what [`crate::interpret::analyze`] returns —
//!   one struct holding both the read side ([`QueryAnalysis`]) and
//!   the write side ([`MutationAnalysis`]) of the document.
//! - [`Application`] captures "predicate applied to terms," shared
//!   between queries and mutations.
//! - [`Statement::Assert`] / [`Statement::Retract`] are the
//!   mutation-side wrappers.
//! - [`Planner::plan`] substitutes query-bound variables in
//!   the parameters of an [`Application`] and produces an
//!   [`ApplicationPlan`] ready for `tx.assert` / `tx.retract`.
//!   The plan is the same shape regardless of which concept
//!   it targets — built-in `attribute` / `concept` are bootstrapped
//!   onto every branch at repo creation, so they resolve like
//!   any other concept.

use std::collections::{HashMap, HashSet};

use dialog_artifacts::{Entity, Statement as ArtifactsStatement, Update, Value};
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use thiserror::Error;

/// Result of analyzing a parsed asserted-notation document.
///
/// One struct, not an enum — a document may contain queries,
/// mutations, or both. Query-only docs leave `mutate.statements`
/// empty; mutation-only docs leave `query` as `None`.
///
/// See `analysis-spec.md` for the three-phase derivation.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// `.foo` → entity. Bookmark-form heads
    /// (`attribute! foo:`, `concept! foo:`, `person! alice:`).
    /// Substituted at analysis time into both queries and
    /// mutations; kept here for the editor's "you defined these
    /// names" introspection view.
    pub declarations: HashMap<String, Entity>,

    /// `?foo` → entity. Variable-form heads
    /// (`attribute! ?foo:` etc.) where the entity is
    /// content-derived. Used as parameter substitutions when
    /// building the unified query (Phase 2), and merged with
    /// query-bound values when planning mutations (Phase 3).
    pub variables: HashMap<String, Entity>,

    /// Read side. `None` for pure-mutation documents.
    pub query: Option<QueryAnalysis>,

    /// Write side. `mutate.statements` is empty for pure-query
    /// documents.
    pub mutate: MutationAnalysis,
}

// ---------------------------------------------------------------- //
// Read side                                                        //
// ---------------------------------------------------------------- //

/// Per-source-expression `Application`s, in document order, with
/// `declarations` and `variables` already substituted in.
///
/// The renderer uses these to project each match back into the
/// user's view ("for the `person ?alice:` expression, here are
/// the matches"). The unified [`ConceptQuery`] the engine
/// evaluates is derived on demand via
/// `ConceptQuery::from(&query_analysis)`.
#[derive(Debug, Clone, Default)]
pub struct QueryAnalysis {
    /// One [`Application`] per source expression.
    pub queries: Vec<Application>,
}

impl QueryAnalysis {
    /// Names of `Term::Variable` slots that survived `variables`
    /// substitution — i.e., what this query binds at evaluation
    /// time.
    pub fn bindings(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for application in &self.queries {
            collect_variable_names(application.parameters(), &mut out);
        }
        out
    }
}

impl From<&QueryAnalysis> for ConceptQuery {
    /// Combine every `queries[i]` predicate's `with` map into one
    /// unified [`ConceptQuery`] whose `terms` union the
    /// per-expression terms. Shared variable names join the
    /// expressions (a `?alice` in two queries means matches must
    /// agree on `alice`).
    ///
    /// The result is what the engine evaluates once per request.
    /// Each per-expression `Application` is also kept on
    /// [`QueryAnalysis::queries`] for per-source rendering.
    fn from(query: &QueryAnalysis) -> Self {
        use dialog_query::{AttributeDescriptor, ConceptDescriptor};

        let mut fields: Vec<(String, AttributeDescriptor)> = Vec::new();
        let mut terms = Parameters::new();
        let mut seen_fields: HashSet<String> = HashSet::new();
        for application in &query.queries {
            let inner = match application {
                Application::Concept { query, .. } => query.clone(),
                Application::Domain { application, .. } => ConceptQuery::from(application.clone()),
            };
            for (name, attribute) in inner.predicate.with().iter() {
                if seen_fields.insert(name.to_owned()) {
                    fields.push((name.to_owned(), attribute.clone()));
                }
            }
            for (name, term) in inner.terms.iter() {
                terms.insert(name.clone(), term.clone());
            }
        }
        ConceptQuery {
            terms,
            predicate: ConceptDescriptor::from(fields),
        }
    }
}

// ---------------------------------------------------------------- //
// Write side                                                       //
// ---------------------------------------------------------------- //

/// Document order. Each `Application` has had `.bookmark`
/// references substituted to constants but keeps `?var`
/// references as variables — substitution happens at planning
/// time.
#[derive(Debug, Clone, Default)]
pub struct MutationAnalysis {
    /// In document order.
    pub statements: Vec<Statement>,
    /// Variable names this plan reads from query bindings.
    /// Disjoint from `Analysis::variables.keys()` (the analyzer
    /// enforces). Subset of `query.bindings()` (the analyzer
    /// also enforces).
    pub requires: HashSet<String>,
}

/// One element of [`MutationAnalysis::statements`] — either an
/// assertion or a retraction of an [`Application`].
#[derive(Debug, Clone)]
pub enum Statement {
    /// `head! …:` — write the facts.
    Assert(Application),
    /// `head! …: _` (or `field: _`) — dissociate matching facts.
    Retract(Application),
}

impl Statement {
    /// The wrapped [`Application`], regardless of variant.
    pub fn application(&self) -> &Application {
        match self {
            Self::Assert(a) | Self::Retract(a) => a,
        }
    }
}

// ---------------------------------------------------------------- //
// Shared between read and write sides                              //
// ---------------------------------------------------------------- //

/// Predicate plus terms plus the source-form binding the head
/// carried. Shared between queries and mutations because both
/// express "a predicate applied to specific terms" — only the
/// consumer differs.
///
/// [`HeadBinding`] is the structural intent (anonymous,
/// variable, bookmark, or URI); `terms["this"]` is computed
/// from it, but the binding is the source of truth. Going
/// `Application` → surface syntax reads it directly. The
/// planner uses it to decide whether to also emit a
/// `dialog.meta/name` claim (bookmark form).
#[derive(Debug, Clone)]
pub enum Application {
    /// `person …:` head — resolved concept with applied terms.
    Concept {
        /// `{ predicate, terms }` ready for evaluation. `terms`
        /// includes a `"this"` slot derived from `binding`.
        query: ConceptQuery,
        /// Source-form binding the head carried.
        binding: HeadBinding,
    },
    /// `xyz.tonk …:` head — claim domain with applied terms;
    /// descriptor is synthesized at planning time from
    /// `application.parameters`.
    Domain {
        /// The domain + parameter map.
        application: DomainApplication,
        /// Source-form binding the head carried.
        binding: HeadBinding,
    },
}

impl Application {
    /// Parameters carried by this application — `Concept` reads
    /// from the inner [`ConceptQuery::terms`], `Domain` from
    /// [`DomainApplication::parameters`].
    pub fn parameters(&self) -> &Parameters {
        match self {
            Self::Concept { query, .. } => &query.terms,
            Self::Domain { application, .. } => &application.parameters,
        }
    }

    /// The source-form binding the head carried.
    pub fn binding(&self) -> &HeadBinding {
        match self {
            Self::Concept { binding, .. } | Self::Domain { binding, .. } => binding,
        }
    }

    /// Variable names appearing in `Term::Variable { name: Some(_) }`
    /// slots of this application's parameters.
    pub fn bindings(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        collect_variable_names(self.parameters(), &mut out);
        out
    }
}

/// Source-form head binding. Mirrors [`tonk_notation::Binding`]
/// 1-to-1 so an [`Application`] round-trips back to surface
/// syntax. The relationship between `binding` and
/// `terms["this"]`:
///
/// - `Anonymous` → `Term::Constant(Entity::of(&body))`
/// - `Variable(name)` bound by query → `Term::Variable(name)`
/// - `Variable(name)` unbound by query →
///   `Term::Constant(<derived entity>)`, with `name` registered
///   in `Analysis::variables`
/// - `Bookmark(name)` → `Term::Constant(Entity::of(&body))`,
///   plus a `dialog.meta/name = name` claim emitted by the
///   planner
/// - `Uri(entity)` → `Term::Constant(Value::Entity(entity))`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadBinding {
    /// `person!:` — no binding token. Entity is body-derived.
    Anonymous,
    /// `person! ?alice:` — variable. Bound by query if some
    /// preceding query expression names `?alice`; otherwise
    /// the analyzer mints a body-derived entity and registers
    /// `alice` in `Analysis::variables`.
    Variable(String),
    /// `person! alice:` — bookmark (git-tag semantics). The
    /// entity is body-derived (so re-running the same body is
    /// a no-op) and the planner emits a `dialog.meta/name`
    /// claim so future docs can resolve `.alice` back to it.
    /// Cardinality-one on `dialog.meta/name` means re-running
    /// with a different body retracts the prior name claim
    /// and binds the name to the new entity.
    Bookmark(String),
    /// `person! did:key:zX:` — explicit entity URI.
    Uri(Entity),
}

/// `xyz.tonk …:` head — claim domains have no schema, so the
/// descriptor is synthesized at planning time from `parameters`.
#[derive(Debug, Clone)]
pub struct DomainApplication {
    /// The claim domain prefix (`xyz.tonk`).
    pub domain: String,
    /// Field-name → term. Each parameter becomes a
    /// `<domain>/<field>` attribute on the synthesized
    /// descriptor (cardinality `one`, no value-type constraint).
    pub parameters: Parameters,
}

impl From<DomainApplication> for ConceptQuery {
    /// Synthesize a [`dialog_query::ConceptDescriptor`] with one
    /// `<domain>/<key>` attribute per parameter (no value-type
    /// constraint) and apply `parameters` to it.
    fn from(d: DomainApplication) -> Self {
        use dialog_query::{
            AttributeDescriptor, Cardinality as DialogCardinality, ConceptDescriptor,
            attribute::The,
        };

        let mut entries: Vec<(String, AttributeDescriptor)> = Vec::new();
        for name in d.parameters.keys() {
            if name == "this" {
                continue;
            }
            let uri = format!("{}/{}", d.domain, name);
            let the: The = uri
                .parse()
                .expect("DomainApplication parameters were validated at analysis time");
            entries.push((
                name.clone(),
                AttributeDescriptor::new(the, "", DialogCardinality::default(), None),
            ));
        }
        ConceptQuery {
            terms: d.parameters,
            predicate: ConceptDescriptor::from(entries),
        }
    }
}

// ---------------------------------------------------------------- //
// Planner                                                          //
// ---------------------------------------------------------------- //

/// Substitute `Term::Variable` slots in an [`Application`]
/// against a binding frame and dispatch on the predicate's
/// identity to produce a typed [`ApplicationPlan`].
pub trait Planner {
    /// The plan type produced.
    type Output;
    /// Substitute `Term::Variable(name)` slots in `self`'s
    /// parameters using `bindings[name]`, then dispatch on the
    /// predicate's identity to produce a typed `Output`. Errors
    /// when a referenced variable isn't bound.
    fn plan(self, bindings: &Parameters) -> Result<Self::Output, PlanError>;
}

/// Reasons [`Planner::plan`] can fail.
#[derive(Debug, Error)]
pub enum PlanError {
    /// A `Term::Variable(name)` had no entry in `bindings`.
    #[error("unbound variable {name:?} — not in query bindings or analysis-time variables")]
    UnboundVariable {
        /// The variable name (without `?` prefix).
        name: String,
    },
}

impl Planner for Application {
    type Output = ApplicationPlan;

    fn plan(self, bindings: &Parameters) -> Result<ApplicationPlan, PlanError> {
        let (query, binding) = match self {
            Self::Concept { query, binding } => (query, binding),
            Self::Domain {
                application,
                binding,
            } => (ConceptQuery::from(application), binding),
        };
        Ok(ApplicationPlan {
            statement: substitute_concept_query(query, bindings)?,
            binding,
        })
    }
}

/// Fully concrete, ready to commit. Wraps a [`ConceptQuery`]
/// whose every `Term::Variable` has been substituted to
/// `Term::Constant` against the planning bindings, plus the
/// source-form binding so the emitter knows whether to also
/// write a `dialog.meta/name` claim (bookmark form).
///
/// Asserting / retracting walks the predicate's `with` map and
/// emits one EAV per non-blank field — exactly the same
/// machinery whether the predicate is the built-in `attribute`
/// schema, the built-in `concept` schema, or a user-defined
/// concept.
pub struct ApplicationPlan {
    /// The substituted query.
    pub statement: ConceptQuery,
    /// Source-form binding the head carried.
    pub binding: HeadBinding,
}

impl ArtifactsStatement for ApplicationPlan {
    fn assert(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, true);
        if let HeadBinding::Bookmark(name) = &self.binding
            && let Some(this) = entity_of_this(&self.statement.terms)
        {
            update.associate(meta_name_attr(), this, Value::String(name.clone()));
        }
    }
    fn retract(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, false);
        if let HeadBinding::Bookmark(name) = &self.binding
            && let Some(this) = entity_of_this(&self.statement.terms)
        {
            update.dissociate(meta_name_attr(), this, Value::String(name.clone()));
        }
    }
}

fn entity_of_this(terms: &Parameters) -> Option<Entity> {
    match terms.get("this")? {
        Term::Constant(Value::Entity(e)) => Some(e.clone()),
        _ => None,
    }
}

fn meta_name_attr() -> dialog_artifacts::Attribute {
    "dialog.meta/name"
        .parse()
        .expect("dialog.meta/name is a valid attribute URI")
}

// ---------------------------------------------------------------- //
// Helpers                                                          //
// ---------------------------------------------------------------- //

fn substitute_concept_query(
    mut query: ConceptQuery,
    bindings: &Parameters,
) -> Result<ConceptQuery, PlanError> {
    let mut new_terms = Parameters::new();
    for (name, term) in query.terms.iter() {
        let resolved = match term {
            Term::Variable {
                name: Some(var_name),
                ..
            } => {
                let Some(bound) = bindings.get(var_name) else {
                    return Err(PlanError::UnboundVariable {
                        name: var_name.clone(),
                    });
                };
                bound.clone()
            }
            other => other.clone(),
        };
        new_terms.insert(name.clone(), resolved);
    }
    query.terms = new_terms;
    Ok(query)
}

fn collect_variable_names(params: &Parameters, out: &mut HashSet<String>) {
    for (_, term) in params.iter() {
        if let Term::Variable {
            name: Some(name), ..
        } = term
        {
            out.insert(name.clone());
        }
    }
}

/// Walk a substituted [`ConceptQuery`] and emit one
/// `(attribute, this, value)` per non-blank parameter — used by
/// `assert` and `retract` on an [`ApplicationPlan`].
///
/// Skips `this` (it's the entity, not a field). Values come
/// from `Term::Constant` slots; `Term::Variable` slots were
/// already substituted by the planner. Blank terms (`_`) are
/// skipped on assert and skipped on retract — retract treats
/// only fields with concrete values as targets.
fn emit_predicate_facts<U: Update>(query: &ConceptQuery, update: &mut U, assert: bool) {
    let Some(this) = query.terms.get("this") else {
        return;
    };
    let this_entity = match this {
        Term::Constant(Value::Entity(e)) => e.clone(),
        _ => return,
    };
    for (field_name, attribute) in query.predicate.with().iter() {
        let Some(term) = query.terms.get(field_name) else {
            continue;
        };
        let Term::Constant(value) = term else {
            continue;
        };
        let the: dialog_artifacts::Attribute = attribute.the().clone().into();
        if assert {
            update.associate(the, this_entity.clone(), value.clone());
        } else {
            update.dissociate(the, this_entity.clone(), value.clone());
        }
    }
}
