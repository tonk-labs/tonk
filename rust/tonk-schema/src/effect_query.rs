//! Effect storage, lookup, and install-time validation.
//!
//! [`effect`](crate::effect) holds the pure [`Effect`] data type
//! and its storage-shape projections. This module holds the
//! interpretation side: loading effects back from a branch,
//! validating the V1 transient-trigger requirement, and writing
//! effects into a branch transaction.
//!
//! This is the effect-lookup machinery the `induce` fixpoint
//! uses. It is an upper layer over the schema definition modules
//! (`concept`, `query_source`, …) — it may freely depend on them.
//! The schema definition layer must NOT depend on this module;
//! the `rule:` query that surfaces installed rules as concept
//! rows lives separately in [`rule_query`](crate::rule_query).
//!
//! Splitting this out keeps `effect.rs` a leaf module that only
//! depends on `dialog-*` crates, so the operation types it
//! defines can move into their own crate later.

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::{Output as _, Term};
use dialog_repository::{Branch, RemoteSite};
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
// Writing effects into a branch transaction.                       //
// ---------------------------------------------------------------- //

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
}
