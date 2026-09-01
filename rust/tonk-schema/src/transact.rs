//! Operation types the analysis tree's nodes hold — the predicate
//! application, the mutation statement, and the planner that turns
//! a statement into committable claims.
//!
//! Quick orientation:
//!
//! - [`Application`] captures "predicate applied to terms," shared
//!   between queries and mutations. [`Application::Rule`] is the
//!   rule-install / rule-retract counterpart: a rule is not a
//!   generic concept (its storage shape is the `db.effect/*`
//!   claims, not a per-attribute `db.concept.with/*` map), so
//!   it gets its own variant.
//! - [`Statement::Assert`] / [`Statement::Retract`] are the
//!   write-direction wrappers. A rule install is
//!   `Statement::Assert(Application::Rule(..))`; a rule retract is
//!   `Statement::Retract(Application::Rule(..))`. There is no
//!   dedicated install/retract variant — the direction lives on
//!   the outer [`Statement`].
//! - [`Planner::plan`] substitutes query-bound variables in
//!   the parameters of an [`Application`] and produces an
//!   [`ApplicationPlan`] ready for `tx.assert` / `tx.retract`.
//!   For [`Application::Rule`] there's nothing to substitute —
//!   the rule already carries its resolved [`Effect`] and the
//!   stored source bytes; the planner just hands it through.

use std::collections::{BTreeMap, HashSet};

use dialog_artifacts::{Entity, Statement as ArtifactsStatement, Update, Value};
use dialog_query::attribute::Relation;
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use thiserror::Error;

use crate::prelude::EntityExt;
use crate::rule::{DeductiveRule, InductiveRule};
use dialog_query::ResolverQuery;
use indexmap::IndexMap;
use serde::Serialize;
use tonk_core::claim::{ConceptDescriptor, PredicateApplication, ValueMap};
use tonk_core::meta::{AnchorName, Name, name};

/// Project a wire-format [`PredicateApplication`] into the
/// concept-shaped [`ApplicationPlan`] the dialog emitter consumes.
/// Used by `/transact` to bridge the wire-side `Claim` batch into
/// the same plan shape the notation path produces.
///
/// If the wire payload omits `"this"`, the subject entity is
/// derived from the predicate and the remaining payload via
/// [`derive_this`] — same hash recipe as the notation path's
/// `Derived` lowering, so identity converges across paths.
pub fn application_plan_from_predicate(application: PredicateApplication) -> ApplicationPlan {
    let (predicate, mut parameters, name) = application.into_parts();
    let descriptor = match predicate {
        ConceptDescriptor::Durable(c) | ConceptDescriptor::Transient(c) => c,
    };
    if !parameters.contains_key("this") {
        let this = derive_this(&descriptor.this(), &parameters);
        parameters.insert("this".into(), Value::Entity(this));
    }
    let mut terms = Parameters::new();
    for (key, value) in parameters {
        terms.insert(key, Term::Constant(value));
    }
    ApplicationPlan::Concept(Box::new(ConceptPlan {
        statement: ConceptQuery {
            terms,
            predicate: descriptor,
        },
        name,
    }))
}

/// Hash a `(predicate, payload)` pair to the entity URI that
/// identifies the assertion's subject. Mirrors the shape the
/// notation analyzer uses for `ThisIntent::Derived` so a `view!`
/// assertion in YAML and a `new-view` command rule end up
/// referring to the same entity when their payloads coincide.
///
/// The hash input is the `{assert: <predicate>, where: {…}}`
/// shape dialog uses elsewhere for predicate application
/// (e.g. constraint serialization), so the digest reads naturally
/// as "the entity identified by this assertion."
pub fn derive_this(predicate: &Entity, payload: &ValueMap) -> Entity {
    Entity::of(&DigestInput {
        assert: predicate,
        payload,
    })
}

/// Serialization shape fed to [`Entity::of`] for deriving the
/// subject entity. `assert` carries the predicate's identity (a
/// concept's `concept:<hash>` URI, or a claim domain string); the
/// `where` map carries the resolved payload. dag-cbor canonicalizes
/// map keys, so insertion order in `payload` does not affect the
/// resulting entity.
#[derive(Serialize)]
struct DigestInput<'a, P: Serialize> {
    assert: &'a P,
    #[serde(rename = "where")]
    payload: &'a IndexMap<String, Value>,
}

/// One lowered write — an assertion or a retraction of an
/// [`Application`]. A rule install is
/// `Statement::Assert(Application::Rule(..))`; a rule retract is
/// `Statement::Retract(Application::Rule(..))`.
#[derive(Debug, Clone)]
pub enum Statement {
    /// `head! …:` — write the facts.
    Assert(Application),
    /// `head! …: _` (or `field: _`) — dissociate matching facts.
    Retract(Application),
}

impl Statement {
    /// The wrapped [`Application`], regardless of direction.
    pub fn application(&self) -> &Application {
        match self {
            Self::Assert(a) | Self::Retract(a) => a,
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
        name: Option<AnchorName>,
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
        name: Option<AnchorName>,
    },
    /// `rule!:` head with the inductive (`assert!:` / `retract!:`)
    /// form — a rule that fires at commit time. Distinct from
    /// `Concept` because a rule's storage shape is dialog's
    /// `dialog.rule/*` claim set, not a per-attribute map.
    ///
    /// For an install, the carried [`InductiveRule`] was built fresh
    /// from the body lift; for a retract it was resolved off the
    /// branch (the canonical encoding makes the dissociate
    /// byte-exact). The outer [`Statement::Assert`] /
    /// [`Statement::Retract`] picks the direction.
    ///
    /// `rule` is boxed because the compiled rule would otherwise
    /// inflate every `Application` variant. The boxed value is
    /// consumed once per claim, so the heap hop is paid at most once
    /// per `rule!:`.
    Rule {
        /// The compiled rule; dialog's `Statement` impl writes the
        /// native `dialog.rule/*` facts.
        rule: Box<InductiveRule>,
        /// Where the retract target entity came from. Rules are
        /// content-addressed, so installs are always `Derived`; a
        /// retract addressed by `this: <entity>` carries `Uri`.
        this: ThisIntent,
    },
    /// `rule!:` head with the deductive (`assert:`, no bang) form —
    /// a rule that derives on query rather than firing on commit.
    /// Same native `dialog.rule/*` storage, minus the trigger index.
    ///
    /// `rule` is boxed for the same reason as [`Application::Rule`].
    DeductiveRule {
        /// The compiled deductive rule.
        rule: Box<DeductiveRule>,
        /// Where the retract target entity came from — `Derived` for
        /// the content-addressed install.
        this: ThisIntent,
    },
    /// A resolver head (`tree/node:`, `tree/entry:`, …) — a premise
    /// dialog answers by content address over immutable blocks rather
    /// than by scanning the mutable head.
    ///
    /// Read-only by construction: a resolver describes what storage
    /// already holds, so there is nothing to assert. It reaches the
    /// query path so the tree inspector can read the store's own
    /// structure as ordinary query rows.
    Resolver {
        /// The resolver, already validated against dialog's schema.
        query: Box<ResolverQuery>,
        /// The resolver's operand terms.
        ///
        /// Carried alongside the query because `ResolverQuery`
        /// rebuilds its parameter map on every call (its terms are
        /// per-variant struct fields, not a stored map), so there is
        /// nothing to borrow — and [`Application::parameters`] must
        /// return a reference. Callers read this to learn which
        /// variables the expression binds; an empty map here silently
        /// yields no rows.
        terms: Parameters,
    },
}

impl Application {
    /// Parameters carried by this application. `Concept` reads
    /// from the inner [`ConceptQuery::terms`], `Domain` from
    /// [`DomainApplication::parameters`]. [`Application::Rule`]
    /// has no parameters — a rule's payload is its body, not a
    /// term map — so this returns an empty parameter set for it.
    pub fn parameters(&self) -> &Parameters {
        match self {
            Self::Concept { query, .. } => &query.terms,
            Self::Domain { application, .. } => &application.parameters,
            Self::Rule { .. } | Self::DeductiveRule { .. } => empty_parameters(),
            // A resolver's operands ARE its parameters: callers read
            // them to learn which variables the expression binds, so
            // returning an empty set here silently yields no rows.
            Self::Resolver { terms, .. } => terms,
        }
    }

    /// Where the entity in `terms["this"]` was selected from.
    pub fn this(&self) -> &ThisIntent {
        match self {
            Self::Concept { this, .. }
            | Self::Domain { this, .. }
            | Self::Rule { this, .. }
            | Self::DeductiveRule { this, .. } => this,
            // A resolver keys on its own `of:` reference, never on a
            // `this:` entity.
            Self::Resolver { .. } => &ThisIntent::Derived,
        }
    }

    /// Name to publish (`&name`), if any. Rules don't take an
    /// `&anchor`, so always `None` for [`Application::Rule`].
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Concept { name, .. } | Self::Domain { name, .. } => {
                name.as_ref().map(AnchorName::as_str)
            }
            Self::Rule { .. } | Self::DeductiveRule { .. } | Self::Resolver { .. } => None,
        }
    }

    /// Variable names appearing in `Term::Variable { name: Some(_) }`
    /// slots of this application's parameters. Empty for
    /// [`Application::Rule`] (rules carry no terms).
    pub fn bindings(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        collect_variable_names(self.parameters(), &mut out);
        out
    }
}

/// Shared singleton empty [`Parameters`] returned by
/// [`Application::parameters`] for [`Application::Rule`]. A rule
/// has no term map; we hand back a borrow into this rather than
/// fabricating a new empty map per call.
fn empty_parameters() -> &'static Parameters {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<Parameters> = OnceLock::new();
    EMPTY.get_or_init(Parameters::new)
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

/// `xyz.tonk …:` head — claim domains have no schema of their own,
/// so the descriptor is synthesized at planning time from
/// `parameters`, honoring any *declared* attributes the analyzer
/// resolved off the branch.
#[derive(Debug, Clone)]
pub struct DomainApplication {
    /// The claim domain prefix (`xyz.tonk`).
    pub domain: String,
    /// Field-name → term. Each parameter becomes a
    /// `<domain>/<field>` attribute on the synthesized descriptor.
    pub parameters: Parameters,
    /// Field-name → the *declared* attribute descriptor, for
    /// fields whose `<domain>/<field>` attribute is published on
    /// the branch. Resolved at analysis time; the synthesized
    /// descriptor takes cardinality and value type from here, so a
    /// `cardinality: many` attribute accumulates through a domain
    /// head exactly as through a concept head. Fields without an
    /// entry fall back to the schema-less default (cardinality
    /// `one`, no value-type constraint).
    pub attributes: BTreeMap<String, dialog_query::AttributeDescriptor>,
}

impl From<DomainApplication> for ConceptQuery {
    /// Synthesize a [`dialog_query::ConceptDescriptor`] with one
    /// `<domain>/<key>` attribute per parameter — the declared
    /// descriptor when the branch has one, a schema-less default
    /// otherwise — and apply `parameters` to it.
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
            if let Some(declared) = d.attributes.get(name) {
                entries.push((name.clone(), declared.clone()));
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
        // A descriptor must have at least one required field. A
        // domain application with only `this` would yield an empty
        // set, so fall back to a single placeholder field — the
        // predicate is unused for matching in that degenerate case.
        if entries.is_empty() {
            let the: The = format!("{}/_", d.domain)
                .parse()
                .expect("domain is a valid attribute prefix");
            entries.push((
                "_".to_string(),
                AttributeDescriptor::new(the, "", DialogCardinality::default(), None),
            ));
        }
        ConceptQuery {
            terms: d.parameters,
            predicate: ConceptDescriptor::try_from(entries)
                .expect("non-empty entries yield a valid descriptor"),
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
        match self {
            Self::Concept { query, name, .. } => {
                Ok(ApplicationPlan::Concept(Box::new(ConceptPlan {
                    statement: substitute_concept_query(query, bindings)?,
                    name,
                })))
            }
            Self::Domain {
                application, name, ..
            } => Ok(ApplicationPlan::Concept(Box::new(ConceptPlan {
                statement: substitute_concept_query(ConceptQuery::from(application), bindings)?,
                name,
            }))),
            Self::Rule { rule, .. } => Ok(ApplicationPlan::Rule(rule)),
            Self::DeductiveRule { rule, .. } => Ok(ApplicationPlan::DeductiveRule(rule)),
            // Read-only by construction: a resolver describes what
            // storage already holds, so it never reaches the mutation
            // planner. The analyzer only ever builds one for a query
            // head — an assertion with a resolver name resolves as a
            // concept and fails there first.
            Self::Resolver { .. } => unreachable!(
                "a resolver is read-only — it has no mutation plan and \
                 the analyzer never lifts one into an assertion"
            ),
        }
    }
}

/// Fully concrete, ready to commit. The lowered form of an
/// [`Application`] after variable substitution.
///
/// [`ApplicationPlan::Concept`] carries a substituted
/// [`ConceptQuery`] (the per-attribute storage shape used by every
/// concept-like application — built-in `attribute`/`concept`,
/// user-defined concepts, and synthesised domain predicates).
/// [`ApplicationPlan::Rule`] carries the compiled rule whose native
/// [`ArtifactsStatement`] impl emits the `dialog.rule/*` storage
/// shape.
pub enum ApplicationPlan {
    /// Per-attribute concept storage (concept / domain / built-in).
    /// Boxed to keep the enum small: [`ConceptPlan`] now carries the
    /// validated published-name [`Entity`], which (alongside the
    /// `Rule` variant) would otherwise leave a large size gap between
    /// variants.
    Concept(Box<ConceptPlan>),
    /// Native `dialog.rule/*` rule storage. Boxed because the
    /// compiled rule would otherwise inflate every concept-shaped
    /// plan to rule-storage size.
    Rule(Box<InductiveRule>),
    /// Native `dialog.rule/*` deductive-rule storage. Boxed for the
    /// same size reason as [`ApplicationPlan::Rule`].
    DeductiveRule(Box<DeductiveRule>),
}

/// Concept-side [`ApplicationPlan`] payload — a substituted
/// [`ConceptQuery`] plus the optional `&anchor` name to publish.
///
/// Asserting / retracting walks the predicate's `with` map and
/// emits one EAV per non-blank field — exactly the same machinery
/// whether the predicate is the built-in `attribute` schema, the
/// built-in `concept` schema, or a user-defined concept.
pub struct ConceptPlan {
    /// The substituted query.
    pub statement: ConceptQuery,
    /// `&name` published by this expression, if any. Triggers
    /// the desugared `name!` assertion against `id:<name>`.
    pub name: Option<AnchorName>,
}

impl ArtifactsStatement for ApplicationPlan {
    fn assert(self, update: &mut impl Update) {
        match self {
            Self::Concept(plan) => plan.assert(update),
            Self::Rule(rule) => (*rule).assert(update),
            Self::DeductiveRule(rule) => (*rule).assert(update),
        }
    }
    fn retract(self, update: &mut impl Update) {
        match self {
            Self::Concept(plan) => plan.retract(update),
            Self::Rule(rule) => (*rule).retract(update),
            Self::DeductiveRule(rule) => (*rule).retract(update),
        }
    }
}

impl ArtifactsStatement for ConceptPlan {
    fn assert(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, true);
        emit_name_assertion(self.name.as_ref(), &self.statement.terms, update, true);
    }
    fn retract(self, update: &mut impl Update) {
        emit_predicate_facts(&self.statement, update, false);
        emit_name_assertion(self.name.as_ref(), &self.statement.terms, update, false);
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
/// that entity carries a `db.meta/name` claim pointing at the
/// body-derived target. Equivalent to:
///
/// ```yaml
/// name!:
///   this:   id:alice
///   entity: <body-derived target>
/// ```
///
/// Cardinality-one on `db.meta/name` means re-running with a
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
    name: Option<&AnchorName>,
    terms: &Parameters,
    update: &mut U,
    assert: bool,
) {
    use dialog_artifacts::Statement as _;

    let Some(name) = name else {
        return;
    };
    let Some(target) = entity_of_this(terms) else {
        return;
    };
    // The `id:<name>` entity was validated when the `AnchorName` was
    // built, so this conversion is infallible — no re-parse, no skip.
    let claim = Name {
        this: name.into(),
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
        // A collection entry is written under `domain/key`; the key is
        // the literal the assertion named, carried in the field's key
        // operand. An entry with no key has no fact to write.
        let the: dialog_artifacts::Attribute = match attribute.the().attribute() {
            Some(the) => the,
            None => {
                let key = query.terms.get(&Relation::key_operand(field_name));
                let Some(Term::Constant(Value::String(key))) = key else {
                    continue;
                };
                let Ok(the) = attribute.the().entry(key) else {
                    continue;
                };
                the
            }
        };
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
    use tonk_core::claim::SourceApplication;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build an `ApplicationPlan::Concept` for a one-field concept
    /// whose `this` is a constant entity. Used by the anchor-desugar
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
        ApplicationPlan::Concept(Box::new(ConceptPlan {
            statement: ConceptQuery {
                terms,
                predicate: descriptor,
            },
            name: Some(AnchorName::try_from(anchor_name).expect("valid anchor name in test")),
        }))
    }

    /// A cardinality-one `unsigned-integer` field (the column
    /// `width`) must reach the change set as its EAV. Pins the
    /// emit path against the live-branch symptom where a seeded
    /// `width: 12` never lands on the column entity.
    #[dialog_common::test]
    fn it_emits_an_unsigned_integer_field() {
        let descriptor: ConceptDescriptor = serde_json::from_str(
            r#"{
                "with": {
                    "width": { "the": "xyz.tonk.column/width", "as": "UnsignedInteger", "cardinality": "one" }
                }
            }"#,
        )
        .unwrap();
        let target: Entity = "did:key:zCol".parse().unwrap();
        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::Constant(Value::Entity(target.clone())));
        terms.insert("width".into(), Term::Constant(Value::UnsignedInt(12)));
        let plan = ApplicationPlan::Concept(Box::new(ConceptPlan {
            statement: ConceptQuery {
                terms,
                predicate: descriptor,
            },
            name: None,
        }));

        let mut changes = Changes::new();
        plan.assert(&mut changes);

        let width_attr: dialog_artifacts::Attribute = "xyz.tonk.column/width".parse().unwrap();
        let saw_width = changes.into_instructions().into_iter().any(|inst| {
            let artifact = match &inst {
                Instruction::Assert(a) | Instruction::Replace(a) => a,
                Instruction::Retract(_) => return false,
            };
            artifact.the == width_attr
                && artifact.of == target
                && artifact.is == Value::UnsignedInt(12)
        });
        assert!(saw_width, "emit dropped the unsigned-integer width EAV");
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
        let meta_name: dialog_artifacts::Attribute = "db.name/referent".parse().unwrap();

        let mut id_alice_name_claim_count = 0;
        let mut wrong_direction_count = 0;
        for inst in changes.into_instructions() {
            // Cardinality-one fields use `Instruction::Replace`
            // (added in dialog tonk-2026-05-11). The anchor name
            // is `db.name/referent` with cardinality one, so
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
            "expected exactly one (id:alice, db.meta/name, target) claim"
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
        let meta_name: dialog_artifacts::Attribute = "db.name/referent".parse().unwrap();

        let saw_dissociate = changes.into_instructions().into_iter().any(|inst| {
            matches!(
                &inst,
                Instruction::Retract(r)
                    if r.the == meta_name && r.of == id_alice && r.is == Value::Entity(target.clone())
            )
        });
        assert!(
            saw_dissociate,
            "expected (id:alice, db.meta/name, target) dissociation"
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
        let plan = ApplicationPlan::Concept(Box::new(ConceptPlan {
            statement: ConceptQuery {
                terms,
                predicate: descriptor,
            },
            name: None,
        }));

        let mut changes = Changes::new();
        plan.assert(&mut changes);

        let meta_name: dialog_artifacts::Attribute = "db.name/referent".parse().unwrap();
        let saw_meta_name = changes
            .into_instructions()
            .into_iter()
            .any(|inst| matches!(inst, Instruction::Assert(a) if a.the == meta_name));
        assert!(
            !saw_meta_name,
            "anonymous bindings should not emit any db.meta/name claim"
        );
    }

    // -- wire-path entity derivation ------------------------------ //

    fn descriptor_with_text_field(domain: &str, name: &str) -> ConceptDescriptor {
        let json = format!(
            r#"{{ "with": {{ "{name}": {{ "the": "{domain}/{name}", "as": "Text", "cardinality": "one" }} }} }}"#,
        );
        serde_json::from_str(&json).unwrap()
    }

    fn wire_application(
        descriptor: ConceptDescriptor,
        payload: &[(&str, Value)],
    ) -> ApplicationPlan {
        let mut parameters = ValueMap::new();
        for (k, v) in payload {
            parameters.insert((*k).into(), v.clone());
        }
        application_plan_from_predicate(
            SourceApplication {
                predicate: super::ConceptDescriptor::Durable(descriptor),
                parameters,
                name: None,
            }
            .try_into()
            .expect("wire application validates"),
        )
    }

    fn this_of(plan: &ApplicationPlan) -> Entity {
        let ApplicationPlan::Concept(concept) = plan else {
            panic!("expected concept plan");
        };
        match concept.statement.terms.get("this").expect("this present") {
            Term::Constant(Value::Entity(e)) => e.clone(),
            other => panic!("expected entity constant, got {other:?}"),
        }
    }

    /// Same `(predicate, payload)` on the wire path produces the
    /// same derived `this` — the property that lets two unrelated
    /// callers converge on a shared subject without coordinating.
    #[dialog_common::test]
    fn it_derives_same_entity_for_same_predicate_and_payload() {
        let d = descriptor_with_text_field("xyz.tonk.view", "name");
        let plan_a = wire_application(d.clone(), &[("name", Value::String("Basic".into()))]);
        let plan_b = wire_application(d, &[("name", Value::String("Basic".into()))]);
        assert_eq!(this_of(&plan_a), this_of(&plan_b));
    }

    /// Same payload across two distinct predicates produces
    /// *different* derived entities — the predicate identity is
    /// part of the hash preimage, so `view{name:"Basic"}` and
    /// `tile{name:"Basic"}` never collide on `this`.
    #[dialog_common::test]
    fn it_derives_different_entities_for_different_predicates() {
        let view = descriptor_with_text_field("xyz.tonk.view", "name");
        let tile = descriptor_with_text_field("xyz.tonk.tile", "name");
        let plan_v = wire_application(view, &[("name", Value::String("Basic".into()))]);
        let plan_t = wire_application(tile, &[("name", Value::String("Basic".into()))]);
        assert_ne!(this_of(&plan_v), this_of(&plan_t));
    }

    /// Distinct payloads under the same predicate diverge — two
    /// `new-view` commands with different `name` produce distinct
    /// subjects so rules and downstream consumers can address each
    /// independently.
    #[dialog_common::test]
    fn it_derives_different_entities_for_different_payloads() {
        let d = descriptor_with_text_field("xyz.tonk.view", "name");
        let plan_a = wire_application(d.clone(), &[("name", Value::String("Basic".into()))]);
        let plan_b = wire_application(d, &[("name", Value::String("Other".into()))]);
        assert_ne!(this_of(&plan_a), this_of(&plan_b));
    }

    /// A wire payload that supplies `this:` keeps that subject
    /// verbatim — derivation only kicks in when the slot is
    /// absent. This is the path the host uses when a descriptor
    /// field projects an entity URI from the DOM (e.g. a
    /// `data-counter` attribute).
    #[dialog_common::test]
    fn it_preserves_caller_supplied_this() {
        let d = descriptor_with_text_field("xyz.tonk.view", "name");
        let caller_this: Entity = "did:key:zCallerSupplied".parse().unwrap();
        let mut parameters = ValueMap::new();
        parameters.insert("this".into(), Value::Entity(caller_this.clone()));
        parameters.insert("name".into(), Value::String("Basic".into()));
        let plan = application_plan_from_predicate(
            SourceApplication {
                predicate: super::ConceptDescriptor::Durable(d),
                parameters,
                name: None,
            }
            .try_into()
            .expect("wire application validates"),
        );
        assert_eq!(this_of(&plan), caller_this);
    }
}
