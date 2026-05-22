//! Query and resolution machinery for effects.
//!
//! [`effect`](crate::effect) holds the pure [`Effect`] data type
//! and its storage-shape projections. This module holds the
//! interpretation side: loading effects back from a branch,
//! validating the V1 transient-trigger requirement, and the
//! [`AnonymousRuleQuery`] that surfaces installed rules as
//! [`ConceptConclusion`] rows.
//!
//! Splitting this out keeps `effect.rs` a leaf module that only
//! depends on `dialog-*` crates, so the operation types it
//! defines can move into their own crate later.

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Select, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::source::SelectRules;
use dialog_query::{
    Application, ConceptDescriptor, EvaluationError, Match, Output as _, Parameters, Selection,
    Term, the, try_stream,
};
use dialog_repository::{Branch, RemoteSite};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::effect::{Effect, EffectError, EffectPolarity};

/// Look up an effect by its entity URI.
pub fn effect_by_entity(entity: Entity) -> EffectByEntity {
    EffectByEntity { entity }
}

/// Validate the V1 trigger requirement against a branch: at
/// least one positive `when` premise must read a concept marked
/// transient.
///
/// This is the install-time check. Construction
/// ([`Effect::new`], [`Effect::asserting`], etc.) is permissive;
/// whoever installs an effect into a branch is responsible for
/// running this check first.
pub async fn validate_effect<Env: EffectEnv>(
    effect: &Effect,
    branch: &Branch,
    env: &Env,
) -> Result<(), EffectValidationError> {
    use crate::concept::TransientConcept;
    use crate::query_source::Source;

    let concepts = effect.when_concept_entities();
    let source = Source::from(branch);
    for entity in concepts {
        let transient = TransientConcept::is_transient(entity)
            .resolve(&source, env)
            .await
            .map_err(|e| EffectValidationError::Query(format!("{e}")))?;
        if transient {
            return Ok(());
        }
    }
    Err(EffectValidationError::MissingTrigger)
}

/// Reasons an effect fails to install. Distinct from
/// [`EffectError`] because the storage-side failures and
/// branch-side failures have different recovery strategies.
#[derive(Debug, Error)]
pub enum EffectValidationError {
    /// No positive `when` premise reads a transient concept.
    /// V1 requires this so the runtime can skip pull-time
    /// re-evaluation for effects.
    #[error(
        "inductive rule has no transient trigger — V1 effects must \
        include at least one positive `when` premise reading a \
        concept declared `transient: true`"
    )]
    MissingTrigger,
    /// The branch query infrastructure returned an error while
    /// looking up a concept's transient marker.
    #[error("transient lookup failed: {0}")]
    Query(String),
}

// ---------------------------------------------------------------- //
// Loading effects back from a branch.                              //
// ---------------------------------------------------------------- //

/// The well-known marker entity asserted as the value of
/// `dialog.meta/effect` on every effect entity, mirroring how
/// concept entities carry `(?this, dialog.meta/concept,
/// db:concept)`. Lets "all effects on this branch" queries start
/// from a selectable triple.
fn effect_marker_entity() -> Entity {
    "db:effect"
        .parse()
        .expect("`db:effect` is a valid entity URI")
}

/// Trait alias gathering the capability bounds every effect
/// resolver needs. Mirrors `concept::QueryEnv`.
pub trait EffectEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}

impl<T> EffectEnv for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Failures specific to loading an effect from a branch.
#[derive(Debug, Error)]
pub enum EffectLookupError {
    /// The branch query infrastructure returned an error.
    #[error("effect lookup query failed: {0}")]
    Query(String),
    /// The effect entity was found but its persisted form was
    /// inconsistent (missing claims, unparseable source, etc.).
    #[error(transparent)]
    Effect(#[from] EffectError),
}

/// Parse a `domain/name` pair as a typed `The`. The crate's
/// effect attributes are well-formed at compile time.
fn the(domain: &str, name: &str) -> dialog_query::attribute::The {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog.effect attribute name is well-formed")
}

/// Builder for [`effect_by_entity`]. Resolves the
/// `dialog.effect/source` and `dialog.effect/polarity` claims and
/// rehydrates the effect.
pub struct EffectByEntity {
    entity: Entity,
}

impl EffectByEntity {
    /// Resolve the effect against a branch.
    pub async fn resolve<Env: EffectEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Option<Effect>, EffectLookupError> {
        // Fetch source.
        let source_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the("dialog.effect", "source"))
                    .of(Term::from(self.entity.clone()))
                    .is(Term::<String>::var("source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| EffectLookupError::Query(format!("source query failed: {e:?}")))?;

        let Some(source_claim) = source_claims.into_iter().next() else {
            return Ok(None);
        };
        let source = match source_claim.is {
            Value::String(s) => s,
            other => {
                return Err(EffectLookupError::Query(format!(
                    "dialog.effect/source claim was not a string: {other:?}"
                )));
            }
        };

        // Fetch polarity.
        let polarity_claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the("dialog.effect", "polarity"))
                    .of(Term::from(self.entity.clone()))
                    .is(Term::<String>::var("polarity")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| EffectLookupError::Query(format!("polarity query failed: {e:?}")))?;

        let polarity_claim = polarity_claims
            .into_iter()
            .next()
            .ok_or(EffectError::InvalidPolarity)?;
        let polarity_str = match polarity_claim.is {
            Value::String(s) => s,
            _ => return Err(EffectError::InvalidPolarity.into()),
        };
        let polarity = EffectPolarity::parse(&polarity_str).ok_or(EffectError::InvalidPolarity)?;

        let effect = Effect::from_source(&source, polarity)?;
        Ok(Some(effect))
    }
}

/// Look up all effect entities whose `dialog.effect/on` index
/// contains the given attribute. `attribute_name` is the runtime
/// `domain/name` form (what a [`Changes`](dialog_artifacts::Changes)
/// instruction's `Attribute` displays as); this builder wraps it
/// in the `on:` prefix to form the index key.
///
/// The reverse-index query the reactor's evaluator runs per
/// round to find effects whose body could have been affected by
/// a change.
pub fn effects_by_on(attribute_name: &str) -> EffectsByOn {
    let uri = format!("on:{attribute_name}");
    let attribute_entity = uri
        .parse()
        .expect("on:<domain>/<name> is a valid entity URI");
    EffectsByOn { attribute_entity }
}

/// Builder for [`effects_by_on`].
pub struct EffectsByOn {
    attribute_entity: Entity,
}

impl EffectsByOn {
    /// Resolve every effect entity whose `dialog.effect/on`
    /// index includes `attribute_entity`.
    pub async fn resolve<Env: EffectEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Vec<Entity>, EffectLookupError> {
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the("dialog.effect", "on"))
                    .of(Term::<Entity>::var("effect"))
                    .is(Term::<Entity>::from(self.attribute_entity.clone())),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| EffectLookupError::Query(format!("on-index query failed: {e:?}")))?;
        let mut out: Vec<Entity> = Vec::with_capacity(claims.len());
        for claim in claims {
            out.push(claim.of);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

// ---------------------------------------------------------------- //
// AnonymousRuleQuery — yields one row per installed rule.          //
// ---------------------------------------------------------------- //

/// JSON-serialisable view of a rule's definition — the value of an
/// [`AnonymousRuleQuery`] row's synthesised `definition` field.
///
/// Pairs the rule's [`InductiveRuleDescriptor`](dialog_query::InductiveRuleDescriptor)
/// (head conclusion, `when` premises, `unless` premises) with its
/// [`EffectPolarity`] so a `rule:` query surfaces what the rule
/// does, mirroring how `concept:` puts a concept's descriptor in
/// its `source` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDefinition {
    /// The inductive rule descriptor: head concept + premises.
    pub rule: dialog_query::InductiveRuleDescriptor,
    /// Whether the head asserts or retracts when the body matches.
    pub polarity: EffectPolarity,
}

impl RuleDefinition {
    /// Build the definition view from an installed [`Effect`].
    fn from_effect(effect: &Effect) -> Self {
        Self {
            rule: effect.descriptor(),
            polarity: effect.polarity(),
        }
    }
}

/// Custom query application that surfaces every inductive rule
/// installed on a branch as a [`ConceptConclusion`].
///
/// Rules are reified as `dialog.effect/*` facts (see the
/// [`effect`](crate::effect) module docs). This query enumerates
/// every entity carrying the `dialog.meta/effect = db:effect`
/// marker, rehydrates each via [`EffectByEntity`], and
/// materialises its definition into a synthesised `definition`
/// field alongside `this` (the effect entity). It is the
/// rule-side parallel of
/// [`AnonymousConceptQuery`](crate::concept::AnonymousConceptQuery):
/// same [`Application`] shape, same `terms` filter/emit
/// convention.
///
/// `terms` parameter keys map to the user's variable names:
/// `this` (the effect entity) and `definition` (the
/// [`RuleDefinition`] as a JSON string). Constant terms filter;
/// variable terms bind.
#[derive(Debug, Clone)]
pub struct AnonymousRuleQuery {
    /// Term bindings — keys `this` (effect entity) and
    /// `definition` (rule definition as a JSON string).
    pub terms: Parameters,
}

impl AnonymousRuleQuery {
    /// Construct a new query from a parameter map.
    pub fn new(terms: Parameters) -> Self {
        Self { terms }
    }
}

/// Pull a constant entity out of a term — either from a constant
/// term or from a variable the upstream selection already bound.
/// Returns `None` if the term is unbound or absent. Mirrors the
/// `resolve_entity_filter` helper in [`crate::concept`].
fn resolve_entity_filter(term: &Option<Term<dialog_query::Any>>, input: &Match) -> Option<Entity> {
    let t = term.as_ref()?;
    match t {
        Term::Constant(value) => Entity::try_from(value.clone()).ok(),
        Term::Variable { name: Some(_), .. } => {
            let value = input.lookup(t).ok()?;
            Entity::try_from(value).ok()
        }
        Term::Variable { name: None, .. } => None,
    }
}

impl Application for AnonymousRuleQuery {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
    {
        let app = self;
        try_stream! {
            for await each in selection {
                let input = each?;

                let this_term = app.terms.get("this").cloned();
                let definition_term = app.terms.get("definition").cloned();

                let this_filter = resolve_entity_filter(&this_term, &input);

                // Enumerate every effect entity via the
                // `dialog.meta/effect = db:effect` marker. When
                // `this` is a constant the selection is already
                // narrowed to that entity.
                let marker = effect_marker_entity();
                let this_term_for_marker: Term<Entity> = match &this_filter {
                    Some(e) => Term::Constant(Value::Entity(e.clone())),
                    None => Term::var("__rule_query_this"),
                };
                let claims: Vec<dialog_query::Claim> = the!("dialog.meta/effect")
                    .of(this_term_for_marker)
                    .is(marker)
                    .perform(env)
                    .try_vec()
                    .await?;

                for claim in claims {
                    let entity = claim.of.clone();

                    // Rehydrate the effect's source + polarity
                    // straight off the same selection environment.
                    let Some(effect) = load_effect_facts(&entity, env).await? else {
                        continue;
                    };

                    let mut m = input.clone();
                    if let Some(ref t) = this_term {
                        m.bind(t, Value::Entity(entity.clone()))?;
                    }
                    if let Some(ref t) = definition_term {
                        let definition = RuleDefinition::from_effect(&effect);
                        let json = serde_json::to_string(&definition)
                            .map_err(|e| EvaluationError::Store(e.to_string()))?;
                        m.bind(t, Value::String(json))?;
                    }
                    yield m;
                }
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        // `ConceptConclusion`'s fields are private; delegate to
        // dialog's `ConceptQuery::realize`, which only reads
        // `terms` and the match's `this` binding. The `predicate`
        // is unused by `realize` so a stub stands in.
        let synthetic = ConceptQuery {
            terms: self.terms.clone(),
            predicate: ConceptDescriptor::from(
                Vec::<(&str, dialog_query::AttributeDescriptor)>::new(),
            ),
        };
        Application::realize(&synthetic, source)
    }
}

/// Rehydrate an effect from its `dialog.effect/source` and
/// `dialog.effect/polarity` claims read straight off a query
/// selection environment.
///
/// Equivalent to [`EffectByEntity::resolve`] but takes the raw
/// `Provider<Select>` selection env that [`Application::evaluate`]
/// is handed, rather than a `&Branch`. Returns `None` when the
/// entity has no `source` claim (a dangling marker).
async fn load_effect_facts<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<Effect>, EvaluationError>
where
    Env: Provider<Select<'a>> + Provider<SelectRules> + ConditionalSync,
{
    let source_claims: Vec<dialog_query::Claim> = the!("dialog.effect/source")
        .of(Term::<Entity>::from(entity.clone()))
        .is(Term::<String>::var("__rule_query_source"))
        .perform(env)
        .try_vec()
        .await?;
    let Some(source_claim) = source_claims.into_iter().next() else {
        return Ok(None);
    };
    let Value::String(source) = source_claim.is else {
        return Err(EvaluationError::Store(
            "dialog.effect/source claim was not a string".to_string(),
        ));
    };

    let polarity_claims: Vec<dialog_query::Claim> = the!("dialog.effect/polarity")
        .of(Term::<Entity>::from(entity.clone()))
        .is(Term::<String>::var("__rule_query_polarity"))
        .perform(env)
        .try_vec()
        .await?;
    let Some(polarity_claim) = polarity_claims.into_iter().next() else {
        return Err(EvaluationError::Store(
            "missing dialog.effect/polarity claim".to_string(),
        ));
    };
    let Value::String(polarity_str) = polarity_claim.is else {
        return Err(EvaluationError::Store(
            "dialog.effect/polarity claim was not a string".to_string(),
        ));
    };
    let polarity = EffectPolarity::parse(&polarity_str)
        .ok_or_else(|| EvaluationError::Store(format!("invalid polarity {polarity_str:?}")))?;

    let effect = Effect::from_source(&source, polarity)
        .map_err(|e| EvaluationError::Store(format!("effect rehydrate failed: {e}")))?;
    Ok(Some(effect))
}

/// The well-known descriptor for the "rule of rules" head.
///
/// Its `with` map names the marker (`dialog.meta/effect`) and the
/// synthesised `definition` field — enough for the analyzer to
/// project the fields a `rule:` head exposes. The `definition`
/// claim has no real EAV backing; it is produced only by
/// [`AnonymousRuleQuery::evaluate`], which the concept query
/// planner dispatches to whenever it sees this descriptor's
/// `this()`.
pub fn rule_of_rule_descriptor() -> &'static ConceptDescriptor {
    static DESCRIPTOR: std::sync::OnceLock<ConceptDescriptor> = std::sync::OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        serde_json::from_value(serde_json::json!({
            "description": "Every inductive rule installed on a branch.",
            "with": {
                "effect":     { "the": "dialog.meta/effect",     "as": "Entity", "cardinality": "one" },
                "definition": { "the": "dialog.effect/source",   "as": "Text",   "cardinality": "one" }
            }
        }))
        .expect("rule-of-rule descriptor is well-formed")
    })
}

/// Build a runtime [`ArtifactsAttribute`] from a domain + local
/// name. Used by the [`assert_effect`] / [`retract_effect`]
/// statement helpers.
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

/// Write an [`Effect`] into a branch transaction — the marker,
/// the source-of-truth claim, and every derived index claim.
///
/// The inverse of [`retract_effect`]. Lives here rather than as a
/// `dialog_artifacts::Statement` impl on [`Effect`] so the
/// `effect` module stays a leaf depending only on `dialog-*`.
pub fn assert_effect(effect: &Effect, update: &mut impl dialog_artifacts::Update) {
    let this = effect.this();
    let description = effect.descriptor().description.clone();
    let source = effect.source();
    let conclusion = effect.conclusion();
    let polarity = effect.polarity();
    let attributes = effect.on_entities();

    // Marker — `(?this, dialog.meta/effect, db:effect)`.
    update.associate_unique(
        meta_attr("dialog.meta", "effect"),
        this.clone(),
        Value::Entity(effect_marker_entity()),
    );
    // Source-of-truth claim.
    update.associate_unique(
        meta_attr("dialog.effect", "source"),
        this.clone(),
        Value::String(source),
    );
    // Head concept index.
    update.associate_unique(
        meta_attr("dialog.effect", "conclusion"),
        this.clone(),
        Value::Entity(conclusion),
    );
    // Polarity tag.
    update.associate_unique(
        meta_attr("dialog.effect", "polarity"),
        this.clone(),
        Value::String(polarity.as_str().to_owned()),
    );
    // Per-attribute reverse index (cardinality-many).
    for attribute in attributes {
        update.associate(
            meta_attr("dialog.effect", "on"),
            this.clone(),
            Value::Entity(attribute),
        );
    }
    // Optional description, shared with the rest of tonk-schema
    // under `dialog.meta/description`.
    if let Some(description) = description
        && !description.is_empty()
    {
        update.associate_unique(
            meta_attr("dialog.meta", "description"),
            this,
            Value::String(description),
        );
    }
}

/// Retract an [`Effect`]'s facts from a branch transaction — the
/// inverse of [`assert_effect`].
pub fn retract_effect(effect: &Effect, update: &mut impl dialog_artifacts::Update) {
    let this = effect.this();
    let description = effect.descriptor().description.clone();
    let source = effect.source();
    let conclusion = effect.conclusion();
    let polarity = effect.polarity();
    let attributes = effect.on_entities();

    update.dissociate(
        meta_attr("dialog.meta", "effect"),
        this.clone(),
        Value::Entity(effect_marker_entity()),
    );
    update.dissociate(
        meta_attr("dialog.effect", "source"),
        this.clone(),
        Value::String(source),
    );
    update.dissociate(
        meta_attr("dialog.effect", "conclusion"),
        this.clone(),
        Value::Entity(conclusion),
    );
    update.dissociate(
        meta_attr("dialog.effect", "polarity"),
        this.clone(),
        Value::String(polarity.as_str().to_owned()),
    );
    for attribute in attributes {
        update.dissociate(
            meta_attr("dialog.effect", "on"),
            this.clone(),
            Value::Entity(attribute),
        );
    }
    if let Some(description) = description
        && !description.is_empty()
    {
        update.dissociate(
            meta_attr("dialog.meta", "description"),
            this,
            Value::String(description),
        );
    }
}

/// Adapter wrapping an [`Effect`] so a branch transaction can
/// `assert` / `retract` it through the `dialog_artifacts::Statement`
/// trait. [`Statement::InstallEffect`](crate::transact::Statement)
/// carries the bare [`Effect`]; the evaluator wraps it here when
/// committing.
pub struct EffectStatement(pub Effect);

impl dialog_artifacts::Statement for EffectStatement {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        assert_effect(&self.0, update);
    }
    fn retract(self, update: &mut impl dialog_artifacts::Update) {
        retract_effect(&self.0, update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concept::TransientConcept;
    use dialog_query::artifact::{Entity as ArtifactsEntity, Type};
    use dialog_query::attribute::{AttributeDescriptor, Cardinality};
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::formula::Formula;
    use dialog_query::formula::math::Sum;
    use dialog_query::parameters::Parameters;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::the;
    use dialog_query::{InductiveRule, Proposition};

    use crate::effect::EFFECT_SYSTEM;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// `counter` concept with a single `count` field.
    fn counter_head() -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            "count",
            AttributeDescriptor::new(
                the!("counter/count"),
                "",
                Cardinality::One,
                Some(Type::UnsignedInt),
            ),
        )])
    }

    /// `increment` command concept.
    fn increment_concept() -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            "subject",
            AttributeDescriptor::new(
                the!("command/subject"),
                "",
                Cardinality::One,
                Some(Type::Entity),
            ),
        )])
    }

    /// Build a `ConceptQuery` premise binding `this` and the
    /// concept's other fields to variables.
    fn concept_premise(predicate: ConceptDescriptor, this: Term<ArtifactsEntity>) -> DialogPremise {
        let mut terms = Parameters::new();
        terms.insert("this".to_string(), this.into());
        for field in predicate.with().iter().map(|(name, _)| name.to_string()) {
            terms.insert(field.clone(), Term::var(&field));
        }
        DialogPremise::Assert(Proposition::Concept(ConceptQuery { terms, predicate }))
    }

    /// Helper: rebind a concept premise's named field. Lets
    /// `counter_head()` read its `count` into `?prev`.
    fn rename_field(p: DialogPremise, from: &str, to: &str) -> DialogPremise {
        match p {
            DialogPremise::Assert(Proposition::Concept(mut cq)) => {
                if cq.terms.get(from).is_some() {
                    cq.terms.insert(from.to_string(), Term::var(to));
                }
                DialogPremise::Assert(Proposition::Concept(cq))
            }
            other => other,
        }
    }

    /// Body for an increment-counter rule: read current counter,
    /// read an increment command on the local sentinel, sum.
    fn increment_body() -> Vec<DialogPremise> {
        let mut sum_terms = Parameters::new();
        sum_terms.insert("of".to_string(), Term::var("prev"));
        sum_terms.insert("with".to_string(), Term::constant(1u64));
        sum_terms.insert("is".to_string(), Term::var("count"));
        vec![
            rename_field(
                concept_premise(counter_head(), Term::<ArtifactsEntity>::var("this")),
                "count",
                "prev",
            ),
            concept_premise(
                increment_concept(),
                Term::Constant(Value::Entity(EFFECT_SYSTEM.clone())),
            ),
            Sum::apply(sum_terms)
                .expect("Sum::apply should succeed")
                .into(),
        ]
    }

    #[dialog_common::test]
    fn it_emits_expected_facts_when_asserted() {
        use dialog_artifacts::{Changes, Instruction};

        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let this = effect.this();
        let source = effect.source();
        let conclusion = effect.conclusion();
        let on_set = effect.on_entities();

        let mut changes = Changes::default();
        assert_effect(&effect, &mut changes);

        let asserted: Vec<_> = changes
            .into_instructions()
            .into_iter()
            .filter_map(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => Some(a),
                Instruction::Retract(_) => None,
            })
            .collect();

        let marker = effect_marker_entity();
        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.meta/effect"
                    && c.of == this
                    && matches!(&c.is, Value::Entity(e) if *e == marker)
            }),
            "missing dialog.meta/effect marker"
        );

        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/source"
                    && c.of == this
                    && matches!(&c.is, Value::String(s) if s == &source)
            }),
            "missing dialog.effect/source claim"
        );

        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/conclusion"
                    && c.of == this
                    && matches!(&c.is, Value::Entity(e) if *e == conclusion)
            }),
            "missing dialog.effect/conclusion claim"
        );

        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/polarity"
                    && c.of == this
                    && matches!(&c.is, Value::String(s) if s == "assert")
            }),
            "missing dialog.effect/polarity claim"
        );

        for attribute in &on_set {
            assert!(
                asserted.iter().any(|c| {
                    c.the.to_string() == "dialog.effect/on"
                        && c.of == this
                        && matches!(&c.is, Value::Entity(e) if *e == *attribute)
                }),
                "missing dialog.effect/on claim for {attribute}"
            );
        }
    }

    #[dialog_common::test]
    fn retract_effect_polarity_serializes_as_retract() {
        use dialog_artifacts::{Changes, Instruction};

        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::retracting(rule);
        let this = effect.this();

        let mut changes = Changes::default();
        assert_effect(&effect, &mut changes);

        let asserted: Vec<_> = changes
            .into_instructions()
            .into_iter()
            .filter_map(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => Some(a),
                _ => None,
            })
            .collect();

        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/polarity"
                    && c.of == this
                    && matches!(&c.is, Value::String(s) if s == "retract")
            }),
            "missing dialog.effect/polarity = retract claim"
        );
    }

    /// V1 install-time check accepts an effect whose body has at
    /// least one positive `when` premise reading a transient
    /// concept. Asserts the `increment` concept as transient on
    /// the branch first; the increment-counter effect's body
    /// reads it.
    #[dialog_common::test]
    async fn it_accepts_effect_with_transient_premise() -> anyhow::Result<()> {
        use dialog_repository::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Mark `increment` as a transient concept on the branch.
        let increment_transient = TransientConcept::new(increment_concept());
        branch
            .transaction()
            .assert(increment_transient)
            .commit()
            .perform(&operator)
            .await?;

        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        validate_effect(&effect, &branch, &operator).await?;
        Ok(())
    }

    /// V1 install-time check rejects an effect whose body has no
    /// positive `when` premise reading a transient concept.
    /// Without asserting `increment` as transient on the branch,
    /// neither of the body's two concept premises (counter,
    /// increment) is marked transient, so validation fails.
    #[dialog_common::test]
    async fn it_rejects_effect_without_transient_premise() -> anyhow::Result<()> {
        use dialog_repository::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);

        match validate_effect(&effect, &branch, &operator).await {
            Err(EffectValidationError::MissingTrigger) => Ok(()),
            other => panic!("expected MissingTrigger, got {other:?}"),
        }
    }

    /// Install an effect on a branch, then query `rule:` via
    /// [`AnonymousRuleQuery`] and assert it surfaces the rule with
    /// its definition. The `definition` field round-trips to a
    /// [`RuleDefinition`] whose `rule` descriptor and `polarity`
    /// match the installed effect — the rule-side parallel of
    /// `concept.rs`'s `it_returns_concept_with_source_from_concept_query`.
    #[dialog_common::test]
    async fn it_enumerates_installed_rules_via_rule_query() -> anyhow::Result<()> {
        use dialog_query::{Any, Output as _, Parameters, Term};
        use dialog_repository::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Install one assert-polarity effect on the branch.
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let effect_entity = effect.this();
        let expected = effect.descriptor();
        branch
            .transaction()
            .assert(EffectStatement(effect.clone()))
            .commit()
            .perform(&operator)
            .await?;

        // Query `rule:` with `this` and `definition` as variables.
        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));
        terms.insert("definition".to_string(), Term::<Any>::var("definition"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousRuleQuery::new(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        // Exactly one rule on the branch — the one we installed.
        assert_eq!(
            conclusions.len(),
            1,
            "expected one rule row; saw {conclusions:?}"
        );
        let row = &conclusions[0];

        // `this` binds the effect entity.
        let this: Entity = Entity::try_from(row.source().lookup(&Term::<Any>::var("this"))?)
            .expect("this binding must be an entity");
        assert_eq!(this, effect_entity);

        // `definition` carries the JSON-serialised RuleDefinition.
        let definition_json: String =
            String::try_from(row.source().lookup(&Term::<Any>::var("definition"))?)
                .expect("definition binding must be a string");
        let definition: RuleDefinition = serde_json::from_str(&definition_json)?;
        assert_eq!(definition.polarity, EffectPolarity::Assert);
        assert_eq!(definition.rule.assert.this(), expected.assert.this());
        assert_eq!(definition.rule.when.len(), expected.when.len());
        assert_eq!(definition.rule.unless.len(), expected.unless.len());
        Ok(())
    }
}
