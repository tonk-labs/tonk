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

/// Predicate plus parameters. Shared between queries and
/// mutations because both express "a predicate applied to
/// specific terms" — only the consumer differs.
///
/// `Concept` carries a [`ConceptQuery`] (dialog's
/// `{ predicate: ConceptDescriptor, terms: Parameters }`), built
/// by resolving the concept against the branch (or in-document
/// state) and applying the user's terms. `Domain` carries a
/// [`DomainApplication`] for `xyz.tonk …:` heads — synthesized
/// inline because claim domains have no schema to look up.
#[derive(Debug, Clone)]
pub enum Application {
    /// Resolved concept head with applied terms.
    Concept(ConceptQuery),
    /// Claim domain head with applied terms — descriptor is
    /// synthesized at planning time from the parameter set.
    Domain(DomainApplication),
}

impl Application {
    /// Parameters carried by this application — `Concept` reads
    /// from the inner [`ConceptQuery::terms`], `Domain` from
    /// [`DomainApplication::parameters`].
    pub fn parameters(&self) -> &Parameters {
        match self {
            Self::Concept(c) => &c.terms,
            Self::Domain(d) => &d.parameters,
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
        let query = match self {
            Self::Concept(q) => q,
            Self::Domain(d) => ConceptQuery::from(d),
        };
        Ok(ApplicationPlan {
            statement: substitute_concept_query(query, bindings)?,
        })
    }
}

/// Fully concrete, ready to commit. Wraps a [`ConceptQuery`]
/// whose every `Term::Variable` has been substituted to
/// `Term::Constant` against the planning bindings. Asserting /
/// retracting walks the predicate's `with` map and emits one
/// EAV per non-blank field — exactly the same machinery whether
/// the predicate is the built-in `attribute` schema, the
/// built-in `concept` schema, or a user-defined concept.
pub struct ApplicationPlan {
    /// The substituted query.
    pub statement: ConceptQuery,
}

impl ArtifactsStatement for ApplicationPlan {
    fn assert(self, update: &mut impl Update) {
        emit_predicate_facts(self.statement, update, true);
    }
    fn retract(self, update: &mut impl Update) {
        emit_predicate_facts(self.statement, update, false);
    }
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
fn emit_predicate_facts<U: Update>(query: ConceptQuery, update: &mut U, assert: bool) {
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
