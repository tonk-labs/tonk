//! Operation types the analysis tree's nodes hold — the predicate
//! application, the mutation statement, and the planner that turns
//! a statement into committable claims.
//!
//! Quick orientation:
//!
//! - [`Application`] captures "predicate applied to terms," shared
//!   between queries and mutations.
//! - [`Statement::Assert`] / [`Statement::Retract`] are the
//!   mutation-side wrappers; [`Statement::InstallEffect`] installs
//!   a `rule!:`-lifted effect.
//! - [`Planner::plan`] substitutes query-bound variables in
//!   the parameters of an [`Application`] and produces an
//!   [`ApplicationPlan`] ready for `tx.assert` / `tx.retract`.
//!   The plan is the same shape regardless of which concept
//!   it targets — built-in `attribute` / `concept` are bootstrapped
//!   onto every branch at repo creation, so they resolve like
//!   any other concept.

use std::collections::HashSet;

use dialog_artifacts::{Entity, Statement as ArtifactsStatement, Update, Value};
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use thiserror::Error;

use crate::effect::Effect;
use crate::meta::{Name, name};

/// One lowered write — an assertion, a retraction of an
/// [`Application`], or the installation of an effect lifted from
/// a `rule!:` expression.
#[derive(Debug, Clone)]
pub enum Statement {
    /// `head! …:` — write the facts.
    Assert(Application),
    /// `head! …: _` (or `field: _`) — dissociate matching facts.
    Retract(Application),
    /// `rule!:` — install an inductive effect on the branch. The
    /// `!` marker makes a rule a mutation: evaluating it writes
    /// the `dialog.effect/*` facts (via
    /// `tonk_evaluator::effect_query::assert_effect`) that
    /// the reactor's induce loop reads on every subsequent commit.
    InstallEffect(Effect),
}

impl Statement {
    /// The wrapped [`Application`], if this statement carries one.
    /// `InstallEffect` has no application — it installs an effect
    /// rather than applying a predicate to terms.
    pub fn application(&self) -> Option<&Application> {
        match self {
            Self::Assert(a) | Self::Retract(a) => Some(a),
            Self::InstallEffect(_) => None,
        }
    }
}

// ---------------------------------------------------------------- //
// Shared between read and write sides                              //
// ---------------------------------------------------------------- //

/// Predicate plus terms plus the source-form intent the head
/// carried — entity selection (`this:`) and naming (`&anchor`).
/// Shared between queries and mutations because both express
/// "a predicate applied to specific terms" — only the consumer
/// differs.
///
/// `this` and `anchor` are independent. `terms["this"]` is
/// computed from `this`; the planner reads `anchor` to decide
/// whether to also emit the desugared `name!` assertion.
#[derive(Debug, Clone)]
pub enum Application {
    /// `person …:` head — resolved concept with applied terms.
    Concept {
        /// `{ predicate, terms }` ready for evaluation. `terms`
        /// includes a `"this"` slot derived from `this`.
        query: ConceptQuery,
        /// Where the entity in `terms["this"]` comes from.
        this: ThisIntent,
        /// `&anchor` on the value side, if any. The planner
        /// emits a desugared `name!` assertion against `id:<n>`
        /// for each `Some(n)`.
        name: Option<String>,
    },
    /// `xyz.tonk …:` head — claim domain with applied terms;
    /// descriptor is synthesized at planning time from
    /// `application.parameters`.
    Domain {
        /// The domain + parameter map.
        application: DomainApplication,
        /// Where the entity in `parameters["this"]` comes from.
        this: ThisIntent,
        /// `&anchor` on the value side, if any.
        name: Option<String>,
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

    /// Where the entity in `terms["this"]` was selected from.
    pub fn this(&self) -> &ThisIntent {
        match self {
            Self::Concept { this, .. } | Self::Domain { this, .. } => this,
        }
    }

    /// Name to publish (`&name`), if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Concept { name, .. } | Self::Domain { name, .. } => name.as_deref(),
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

/// How the entity in `terms["this"]` is selected. Mirrors the
/// `this:` body field's value forms. Hoisted onto
/// [`Application`] / [`ApplicationPlan`] alongside an optional
/// anchor, since the two intents are orthogonal: every
/// combination of `this:` and `&anchor` is meaningful.
///
/// Examples:
///
/// - `person!:` — `Derived` + `anchor = None`
/// - `person!:\n  this: ?alice` — `Variable("alice")` + `anchor
///   = None`
/// - `person!: &alice` — `Derived` + `anchor = Some("alice")`
///   (publish `id:alice` pointing at the body-derived entity)
/// - `person!: &alice\n  this: did:key:zX` — `Uri(zX)` +
///   `anchor = Some("alice")` (publish `id:alice` pointing at
///   zX without producing a new entity)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThisIntent {
    /// `this:` omitted. Entity is body-derived
    /// (`Entity::of(&body)`); on queries this surfaces as a
    /// fresh anonymous variable so dialog's engine has a slot
    /// to bind matches into.
    Derived,
    /// `this: ?name`. Bound by query if some preceding query
    /// expression names `?name`; otherwise the analyzer mints a
    /// body-derived entity and registers `name` in
    /// `Analysis::variables`.
    Variable(String),
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
        let (query, name) = match self {
            Self::Concept { query, name, .. } => (query, name),
            Self::Domain {
                application, name, ..
            } => (ConceptQuery::from(application), name),
        };
        Ok(ApplicationPlan {
            statement: substitute_concept_query(query, bindings)?,
            name,
        })
    }
}

/// Fully concrete, ready to commit. Wraps a [`ConceptQuery`]
/// whose every `Term::Variable` has been substituted to
/// `Term::Constant` against the planning bindings, plus the
/// optional name to publish so the emitter knows whether to also
/// emit the desugared `name!` assertion.
///
/// Asserting / retracting walks the predicate's `with` map and
/// emits one EAV per non-blank field — exactly the same
/// machinery whether the predicate is the built-in `attribute`
/// schema, the built-in `concept` schema, or a user-defined
/// concept.
pub struct ApplicationPlan {
    /// The substituted query.
    pub statement: ConceptQuery,
    /// `&name` published by this expression, if any. Triggers
    /// the desugared `name!` assertion against `id:<name>`.
    pub name: Option<String>,
}

impl ArtifactsStatement for ApplicationPlan {
    fn assert(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, true);
        emit_name_assertion(self.name.as_deref(), &self.statement.terms, update, true);
    }
    fn retract(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, false);
        emit_name_assertion(self.name.as_deref(), &self.statement.terms, update, false);
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
fn emit_name_assertion<U: Update>(
    name: Option<&str>,
    terms: &Parameters,
    update: &mut U,
    assert: bool,
) {
    use dialog_artifacts::Statement as _;

    let Some(name_str) = name else {
        return;
    };
    let Some(target) = entity_of_this(terms) else {
        return;
    };
    let Ok(id_entity) = format!("id:{name_str}").parse::<Entity>() else {
        return;
    };
    let claim = Name {
        this: id_entity,
        entity: name::Referent(target),
    };
    if assert {
        claim.assert(update);
    } else {
        claim.retract(update);
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
            name: Some(anchor_name.into()),
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
        let meta_name: dialog_artifacts::Attribute = "dialog.name/referent".parse().unwrap();

        let mut id_alice_name_claim_count = 0;
        let mut wrong_direction_count = 0;
        for inst in changes.into_instructions() {
            // Cardinality-one fields use `Instruction::Replace`
            // (added in dialog tonk-2026-05-11). The anchor name
            // is `dialog.name/referent` with cardinality one, so
            // the desugared `name!` lands as a Replace, not an
            // Assert.
            let artifact = match &inst {
                Instruction::Assert(a) | Instruction::Replace(a) => a,
                Instruction::Retract(_) => continue,
            };
            if artifact.the == meta_name {
                if artifact.of == id_alice && artifact.is == Value::Entity(target.clone()) {
                    id_alice_name_claim_count += 1;
                }
                if artifact.of == target {
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
        let meta_name: dialog_artifacts::Attribute = "dialog.name/referent".parse().unwrap();

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
            name: None,
        };

        let mut changes = Changes::new();
        plan.assert(&mut changes);

        let meta_name: dialog_artifacts::Attribute = "dialog.name/referent".parse().unwrap();
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
