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

use dialog_artifacts::{Entity, Select, Statement as ArtifactsStatement, Update, Value};
use dialog_capability::Provider;
use dialog_common::ConditionalSync;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::source::SelectRules;
use dialog_query::{
    Application as DialogApplication, EvaluationError, Match, Parameters, Selection, Term,
    concept::query::ConceptQuery, try_stream,
};
use thiserror::Error;

use crate::concept::QueryPlan;

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
    /// Names of user-named `Term::Variable` slots that survived
    /// `variables` substitution — i.e., what this query binds at
    /// evaluation time. Auto-generated [`Term::unique`] variables
    /// (whose names start with `__`) are excluded; they're an
    /// implementation detail of anonymous-head bindings, not
    /// user-visible bindings.
    pub fn bindings(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for application in &self.queries {
            collect_user_variable_names(application.parameters(), &mut out);
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

/// Source-form intent for entity selection on a head. Derived
/// from the body's `this:` field and (on assertions) any `&anchor`
/// between the head's colon and the body. The relationship between
/// `binding` and `terms["this"]`:
///
/// - `Anonymous` (no `this:`, no `&anchor`) →
///   `Term::Constant(Entity::of(&body))`
/// - `Variable(name)` (`this: ?name`) bound by query →
///   `Term::Variable(name)`
/// - `Variable(name)` (`this: ?name`) unbound by query →
///   `Term::Constant(<derived entity>)`, with `name` registered
///   in `Analysis::variables`
/// - `Anchor(name)` (`&name` anchor on assertion) →
///   `Term::Constant(Entity::of(&body))`, **plus** a desugared
///   `name!` assertion emitted by the planner against `id:<name>`
///   that points at the body-derived target via `dialog.meta/name`
/// - `Uri(entity)` (`this: did:key:…` / `id:…` / `db:…`) →
///   `Term::Constant(Value::Entity(entity))`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadBinding {
    /// No `this:` field, no `&anchor`. Entity is body-derived.
    Anonymous,
    /// `this: ?name`. Bound by query if some preceding query
    /// expression names `?name`; otherwise the analyzer mints a
    /// body-derived entity and registers `name` in
    /// `Analysis::variables`.
    Variable(String),
    /// `&name` anchor on an assertion's value side (`person!:
    /// &alice`). The asserted entity is body-derived (so
    /// re-running the same body is a no-op); the planner *also*
    /// emits the implicit `name!` assertion that desugars from
    /// the anchor: `(id:<name>, dialog.meta/name, body-entity)`.
    /// Cardinality-one on `dialog.meta/name` means re-running
    /// with a different body replaces the prior `entity:` claim
    /// on `id:<name>`, repointing the name at the new target —
    /// same git-tag semantics, but the EAV hangs off the *name*
    /// entity, not the named one.
    Anchor(String),
    /// `this:` carried a URI directly (`did:key:…`, `id:…`,
    /// `db:…`).
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
        emit_anchor_name_assertion(&self.binding, &self.statement.terms, update, true);
    }
    fn retract(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, false);
        emit_anchor_name_assertion(&self.binding, &self.statement.terms, update, false);
    }
}

fn entity_of_this(terms: &Parameters) -> Option<Entity> {
    match terms.get("this")? {
        Term::Constant(Value::Entity(e)) => Some(e.clone()),
        _ => None,
    }
}

/// Emit the implicit `name!` assertion that an anchored head
/// (`person!: &alice`) desugars to.
///
/// The anchor name `alice` becomes the entity URI `id:alice`, and
/// that entity carries a `dialog.meta/name` claim pointing at the
/// body-derived target. Equivalent to:
///
/// ```yaml
/// name!:
///   this:   id:alice
///   entity: <body-derived target>
/// ```
///
/// Cardinality-one on `dialog.meta/name` means re-running with a
/// different body retracts the prior `entity:` claim and binds the
/// name to the new target — same git-tag semantics, but the EAV
/// hangs off the *name* entity, not the named one.
///
/// Skips silently if the anchor's `id:<name>` URI doesn't parse.
/// In practice every anchor name that survived the parser's symbol
/// charset produces a valid `id:<name>`; the conservative skip
/// keeps a hypothetical bad case from poisoning the surrounding
/// transaction.
fn emit_anchor_name_assertion<U: Update>(
    binding: &HeadBinding,
    terms: &Parameters,
    update: &mut U,
    assert: bool,
) {
    let HeadBinding::Anchor(name) = binding else {
        return;
    };
    let Some(target) = entity_of_this(terms) else {
        return;
    };
    let Ok(id_entity) = format!("id:{name}").parse::<Entity>() else {
        return;
    };
    let attribute = meta_name_attr();
    let value = Value::Entity(target);
    if assert {
        // Cardinality-one: replace, not accumulate.
        update.associate_unique(attribute, id_entity, value);
    } else {
        update.dissociate(attribute, id_entity, value);
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

/// Like [`collect_variable_names`] but skips auto-generated
/// `Term::unique` names (which start with `__`). Used by the
/// component-grouping logic so anonymous-head bindings do not
/// accidentally connect unrelated expressions.
fn collect_user_variable_names(params: &Parameters, out: &mut HashSet<String>) {
    for (_, term) in params.iter() {
        if let Term::Variable {
            name: Some(name), ..
        } = term
            && !name.starts_with("__")
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
    use dialog_query::Cardinality;

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
            // Cardinality-one fields use `associate_unique` so a
            // re-assert of the same attribute on the same entity
            // *replaces* the prior value rather than accumulating
            // multiple claims. Cardinality-many fields stay
            // additive (the whole point is multiple values).
            match attribute.cardinality() {
                Cardinality::One => {
                    update.associate_unique(the, this_entity.clone(), value.clone());
                }
                Cardinality::Many => {
                    update.associate(the, this_entity.clone(), value.clone());
                }
            }
        } else {
            update.dissociate(the, this_entity.clone(), value.clone());
        }
    }
}

// ---------------------------------------------------------------- //
// Read-side evaluation: Application + QueryAnalysis as queries.    //
// ---------------------------------------------------------------- //
//
// Both [`Application`] and [`QueryAnalysis`] are analyzer outputs
// and both impl `dialog_query::Application`. `Application` runs
// one expression at a time (delegating to [`QueryPlan`] so
// built-in heads dispatch transparently). `QueryAnalysis` chains
// every expression's evaluation through a shared selection
// stream, which gives the engine's variable-binding consistency
// check the role of a natural join: matches that disagree on a
// shared user-named variable never reach the conclusion.
//
// Conclusions:
// - `Application::Conclusion = ConceptConclusion` (one entity per
//   row).
// - `QueryAnalysis::Conclusion = QueryNotationConclusion` (a
//   `Parameters` row over every user-named variable).

/// Convert an [`Application`] into the [`QueryPlan`] it should be
/// evaluated as. `Concept` carries a `ConceptQuery` directly;
/// `Domain` synthesises one from its parameter map.
fn application_to_plan(application: Application) -> QueryPlan {
    match application {
        Application::Concept { query, .. } => QueryPlan::from(query),
        Application::Domain { application, .. } => QueryPlan::from(ConceptQuery::from(application)),
    }
}

/// Like [`application_to_plan`] but borrows. Needed for
/// [`DialogApplication::realize`] which receives `&self`.
fn application_to_plan_cloned(application: &Application) -> QueryPlan {
    match application {
        Application::Concept { query, .. } => QueryPlan::from(query.clone()),
        Application::Domain { application, .. } => {
            QueryPlan::from(ConceptQuery::from(application.clone()))
        }
    }
}

impl DialogApplication for Application {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        let plan = application_to_plan(self);
        try_stream! {
            let stream = plan.evaluate(selection, env);
            for await each in stream {
                yield each?;
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        DialogApplication::realize(&application_to_plan_cloned(self), source)
    }
}

/// One joined frame produced by a [`QueryAnalysis`] evaluation.
///
/// A document's `query:` block can hold multiple expressions; each
/// expression contributes user-named variable bindings, and the
/// engine natural-joins them on shared names. This type is the
/// realized form of a single joined row.
#[derive(Debug, Clone)]
pub struct QueryNotationConclusion {
    /// User-named variable bindings carried by this row. Keys are
    /// the user's `?var` names; values are constants.
    pub bindings: Parameters,
}

/// Variable names (`Term::Variable { name: Some(_) }`) appearing
/// in a parameter map, deduplicated.
fn collect_named_variables(params: &Parameters) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, term) in params.iter() {
        if let Term::Variable {
            name: Some(name), ..
        } = term
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    names
}

/// Every user-named variable across every expression in this
/// analysis, deduplicated. Used by the realize step to know which
/// keys to project from the joined match into the conclusion's
/// `bindings`.
fn collect_analysis_variables(analysis: &QueryAnalysis) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for application in &analysis.queries {
        for n in collect_named_variables(application.parameters()) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}

impl DialogApplication for QueryAnalysis {
    type Conclusion = QueryNotationConclusion;

    /// Evaluate every expression in document order, threading the
    /// upstream selection through each. A `Selection` is itself a
    /// stream of `Match` values; chaining `Application::evaluate`
    /// on each expression performs the natural join automatically
    /// because shared variable names re-bind to the same value
    /// (consistency-preserving) and disagreement aborts the row.
    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        try_stream! {
            // Box::pin once per expression so the chained stream
            // type stays sized as the chain grows.
            let mut current: std::pin::Pin<Box<dyn Selection<Item = Result<Match, EvaluationError>> + 'a>> =
                Box::pin(selection);
            for application in self.queries {
                let next = application.evaluate(current, env);
                current = Box::pin(next);
            }
            for await each in current {
                yield each?;
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        let mut bindings = Parameters::new();
        for name in collect_analysis_variables(self) {
            if let Ok(value) = source.lookup(&Term::<dialog_query::Any>::var(name.clone())) {
                bindings.insert(name, Term::Constant(value));
            }
        }
        Ok(QueryNotationConclusion { bindings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::{Changes, Instruction};
    use dialog_query::ConceptDescriptor;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build an `ApplicationPlan` for a one-field concept whose
    /// `this` is a constant entity. Used by the anchor-desugar
    /// tests below.
    fn plan_with_anchor(anchor_name: &str, target: &str) -> ApplicationPlan {
        let descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "x.y/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )
        .unwrap();
        let target_entity: Entity = target.parse().unwrap();
        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::Constant(Value::Entity(target_entity)));
        terms.insert("name".into(), Term::Constant(Value::String("x".into())));
        ApplicationPlan {
            statement: ConceptQuery {
                terms,
                predicate: descriptor,
            },
            binding: HeadBinding::Anchor(anchor_name.into()),
        }
    }

    /// Asserting an anchored plan emits the desugared `name!`
    /// claim on `id:<name>` (not on the body-derived target).
    #[dialog_common::test]
    fn it_emits_anchor_name_assertion_on_id_entity() {
        let target_uri = "did:key:zHjKfTestTarget";
        let plan = plan_with_anchor("alice", target_uri);
        let mut changes = Changes::new();
        plan.assert(&mut changes);

        let id_alice: Entity = "id:alice".parse().unwrap();
        let target: Entity = target_uri.parse().unwrap();
        let meta_name = meta_name_attr();

        let mut id_alice_name_claim_count = 0;
        let mut wrong_direction_count = 0;
        for inst in changes.into_instructions() {
            if let Instruction::Assert(a) = &inst
                && a.the == meta_name
            {
                if a.of == id_alice && a.is == Value::Entity(target.clone()) {
                    id_alice_name_claim_count += 1;
                }
                if a.of == target {
                    wrong_direction_count += 1;
                }
            }
        }
        assert_eq!(
            id_alice_name_claim_count, 1,
            "expected exactly one (id:alice, dialog.meta/name, target) claim"
        );
        assert_eq!(
            wrong_direction_count, 0,
            "expected no claims on the target entity (anchor name lives on id:<name>)"
        );
    }

    /// Retracting an anchored plan dissociates the same EAV the
    /// assert path would have written.
    #[dialog_common::test]
    fn it_retracts_anchor_name_assertion_on_id_entity() {
        let target_uri = "did:key:zHjKfTestTarget";
        let plan = plan_with_anchor("alice", target_uri);
        let mut changes = Changes::new();
        plan.retract(&mut changes);

        let id_alice: Entity = "id:alice".parse().unwrap();
        let target: Entity = target_uri.parse().unwrap();
        let meta_name = meta_name_attr();

        let saw_dissociate = changes.into_instructions().into_iter().any(|inst| {
            matches!(
                &inst,
                Instruction::Retract(r)
                    if r.the == meta_name && r.of == id_alice && r.is == Value::Entity(target.clone())
            )
        });
        assert!(
            saw_dissociate,
            "expected (id:alice, dialog.meta/name, target) dissociation"
        );
    }

    /// Anonymous binding emits no anchor-name claim at all.
    #[dialog_common::test]
    fn it_emits_no_anchor_name_for_anonymous_binding() {
        let descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "name": { "the": "x.y/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        )
        .unwrap();
        let target: Entity = "did:key:zAnon".parse().unwrap();
        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::Constant(Value::Entity(target)));
        terms.insert("name".into(), Term::Constant(Value::String("x".into())));
        let plan = ApplicationPlan {
            statement: ConceptQuery {
                terms,
                predicate: descriptor,
            },
            binding: HeadBinding::Anonymous,
        };

        let mut changes = Changes::new();
        plan.assert(&mut changes);

        let meta_name = meta_name_attr();
        let saw_meta_name = changes
            .into_instructions()
            .into_iter()
            .any(|inst| matches!(inst, Instruction::Assert(a) if a.the == meta_name));
        assert!(
            !saw_meta_name,
            "anonymous bindings should not emit any dialog.meta/name claim"
        );
    }
}
