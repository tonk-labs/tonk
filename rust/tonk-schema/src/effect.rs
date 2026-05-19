//! Storage schema for effects: inductive rules with polarity.
//!
//! An [`Effect`] pairs a compiled
//! [`InductiveRule`](dialog_query::InductiveRule) with a
//! [`Polarity`] saying what happens when the body matches:
//! [`Polarity::Assert`] produces persistent head facts;
//! [`Polarity::Retract`] produces retracts for the matched cells.
//!
//! Effects are reified as facts on a branch so they replicate and
//! are queryable like any other concept. The reactor's evaluator
//! loads stored effects on each commit, fires whatever matches,
//! and produces head facts as part of the transaction.
//!
//! # Storage shape
//!
//! Each effect entity carries:
//!
//! - `dialog.effect/source` — the full
//!   [`InductiveRuleDescriptor`](dialog_query::InductiveRuleDescriptor)
//!   serialized as JSON. Source of truth: anything we need to
//!   re-evaluate the rule can be reconstructed from this single
//!   text claim.
//! - `dialog.effect/conclusion` — index pointing at the head
//!   concept entity, equal to
//!   [`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this).
//!   One claim per effect; the name matches the upstream
//!   `InductiveRule::conclusion()` accessor regardless of
//!   polarity.
//! - `dialog.effect/polarity` — `"assert"` or `"retract"`, the
//!   polarity of the rule's head. Disambiguates two effects with
//!   structurally-identical descriptors but different intent.
//! - `dialog.effect/premise` — index listing every attribute
//!   entity referenced by any premise's predicate (positive
//!   `when` ∪ negative `unless`). Cardinality-many; one claim per
//!   distinct attribute the body reads. The reverse index is
//!   attribute-keyed so it invalidates correctly under
//!   concept-lens-sharing.
//! - `dialog.meta/description` — optional human-readable
//!   description.
//!
//! The index attributes are pure projections of the source.
//!
//! # Transient triggers (V1)
//!
//! V1 requires an effect's body to read at least one premise
//! whose concept is marked transient. The check is enforced at
//! install time (when an effect is loaded against a branch and
//! the transient marker can be queried), not at construction
//! time. [`Effect::new`] succeeds for any compiled rule; the
//! load path is responsible for rejecting effects whose body has
//! no transient trigger.

// `#[derive(Attribute)]` expands to helper items without doc
// comments; suppress the crate-level `missing_docs` lint here.
#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::sync::LazyLock;

use base58::ToBase58;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Update, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::{Blake3Hash, ConditionalSync};
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::{
    Attribute, InductiveRule, InductiveRuleDescriptor, Output as _, Proposition, Statement, Term,
};
use dialog_repository::{Branch, RemoteSite};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Sentinel entity conventionally used as the `this` of ability
/// concepts (the local command bus). Not a load-bearing part of
/// the V1 trigger check anymore — transience is now a property of
/// the concept declaration rather than the URI — but the URI
/// remains a useful well-known anchor for UI-submitted commands.
pub const EFFECT_SYSTEM_URI: &str = "did:key:zEffectSystem";

/// `effect:system` as a parsed [`Entity`].
pub static EFFECT_SYSTEM: LazyLock<Entity> = LazyLock::new(|| {
    EFFECT_SYSTEM_URI
        .parse()
        .expect("EFFECT_SYSTEM_URI is a valid entity URI")
});

// ---------------------------------------------------------------- //
// Schema attributes                                                //
// ---------------------------------------------------------------- //

/// The canonical JSON of an effect's [`InductiveRuleDescriptor`].
/// Source of truth — every other `dialog.effect/*` attribute is a
/// derived index that the reactor recomputes from this claim.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.effect")]
pub struct Source(pub String);

/// The head concept entity referenced by the rule, regardless of
/// polarity. Equal to
/// [`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this)
/// on the rule's
/// [`InductiveRule::conclusion`](dialog_query::InductiveRule::conclusion).
/// One claim per effect.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.effect")]
pub struct Conclusion(pub Entity);

/// `"assert"` or `"retract"`. Distinguishes effects whose head
/// produces new persistent facts from effects whose head
/// produces retracts of existing cells.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.effect")]
pub struct Polarity(pub String);

/// An attribute entity referenced by some premise in the
/// effect's body (positive `when` or negative `unless`).
/// Cardinality-many: one claim per distinct attribute the body
/// reads.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cardinality(many)]
#[domain("dialog.effect")]
pub struct Premise(pub Entity);

// ---------------------------------------------------------------- //
// Effect type                                                      //
// ---------------------------------------------------------------- //

/// Polarity of an effect's head: whether matching the body
/// produces new persistent assertions or retractions of existing
/// cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectPolarity {
    /// The head produces persistent assertions for the named
    /// concept's attributes when the body matches.
    Assert,
    /// The head produces retractions for the matched concept's
    /// attributes when the body matches.
    Retract,
}

impl EffectPolarity {
    /// String form for the `dialog.effect/polarity` claim's
    /// value. Matches the lowercase variant name.
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectPolarity::Assert => "assert",
            EffectPolarity::Retract => "retract",
        }
    }

    /// Parse from the string form stored in
    /// `dialog.effect/polarity`. Returns `None` on unknown
    /// variants.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "assert" => Some(EffectPolarity::Assert),
            "retract" => Some(EffectPolarity::Retract),
            _ => None,
        }
    }
}

/// Reasons an effect cannot be constructed or loaded.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EffectError {
    /// The rule's body does not contain a positive premise reading
    /// a transient concept. V1 effects require this so the runtime
    /// knows the rule only fires on locally-submitted transients
    /// and can skip pull-time re-evaluation. Enforced at install
    /// time when transient markers can be queried from the branch.
    #[error(
        "inductive rule has no transient trigger — V1 effects must \
        include at least one positive `when` premise reading a \
        transient concept"
    )]
    MissingTrigger,

    /// Could not deserialize the effect's `dialog.effect/source`
    /// claim as an [`InductiveRuleDescriptor`].
    #[error("failed to parse stored effect source: {0}")]
    Deserialize(String),

    /// The effect's `dialog.effect/polarity` claim was missing or
    /// unrecognized.
    #[error("missing or invalid dialog.effect/polarity for stored effect")]
    InvalidPolarity,
}

/// An inductive rule + polarity, ready to be installed on a
/// branch. Construction is structural only; the V1 trigger check
/// runs at install time.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    rule: InductiveRule,
    polarity: EffectPolarity,
}

impl Effect {
    /// Construct an effect from a compiled rule and a polarity.
    /// No trigger validation; that's enforced at install time
    /// against the branch.
    pub fn new(rule: InductiveRule, polarity: EffectPolarity) -> Self {
        Self { rule, polarity }
    }

    /// Convenience constructor for an assert-polarity effect.
    pub fn asserting(rule: InductiveRule) -> Self {
        Self::new(rule, EffectPolarity::Assert)
    }

    /// Convenience constructor for a retract-polarity effect.
    pub fn retracting(rule: InductiveRule) -> Self {
        Self::new(rule, EffectPolarity::Retract)
    }

    /// Parse the canonical JSON form of an
    /// [`InductiveRuleDescriptor`] and pair with the given
    /// polarity.
    pub fn from_source(source: &str, polarity: EffectPolarity) -> Result<Self, EffectError> {
        let rule: InductiveRule =
            serde_json::from_str(source).map_err(|e| EffectError::Deserialize(e.to_string()))?;
        Ok(Self::new(rule, polarity))
    }

    /// Borrow the underlying rule.
    pub fn rule(&self) -> &InductiveRule {
        &self.rule
    }

    /// Unwrap into the underlying rule.
    pub fn into_rule(self) -> InductiveRule {
        self.rule
    }

    /// Polarity of this effect.
    pub fn polarity(&self) -> EffectPolarity {
        self.polarity
    }

    /// Reconstruct the serializable descriptor.
    pub fn descriptor(&self) -> InductiveRuleDescriptor {
        self.rule.descriptor()
    }

    /// The effect's content-addressed entity URI. Hash inputs
    /// include both the descriptor and the polarity, so an
    /// assert-version and retract-version of the same body have
    /// distinct URIs.
    pub fn this(&self) -> Entity {
        let descriptor = self.rule.descriptor();
        let payload = (descriptor, self.polarity);
        let bytes = serde_ipld_dagcbor::to_vec(&payload)
            .expect("dag-cbor encoding of effect should not fail");
        let hash = Blake3Hash::hash(&bytes);
        let encoded = hash.as_bytes().as_ref().to_base58();
        format!("effect:{encoded}")
            .parse()
            .expect("effect:<base58> is a valid entity URI")
    }

    /// Canonical JSON form of the rule descriptor — the value of
    /// the `dialog.effect/source` claim.
    pub fn source(&self) -> String {
        serde_json::to_string(&self.rule.descriptor())
            .expect("InductiveRuleDescriptor always serializes to JSON")
    }

    /// The head concept entity — the value of the
    /// `dialog.effect/conclusion` claim.
    pub fn conclusion(&self) -> Entity {
        self.rule.conclusion().this()
    }

    /// The set of attribute entities the body reads from. For
    /// each `Proposition::Concept` premise in `when` or `unless`,
    /// every attribute referenced by the premise's predicate
    /// contributes its `the:<hash>` URI to the set. Values of
    /// the `dialog.effect/premise` claims.
    ///
    /// Attribute-direct premises (`Proposition::Attribute`) are
    /// not currently included; they read individual EAV triples
    /// directly, which the yaml authoring surface doesn't expose.
    ///
    /// Formula premises (`math/sum`, `==`, etc.) contribute
    /// nothing.
    pub fn premise_entities(&self) -> BTreeSet<Entity> {
        let descriptor = self.rule.descriptor();
        let mut entities = BTreeSet::new();
        for proposition in descriptor.when.iter().chain(descriptor.unless.iter()) {
            if let Proposition::Concept(concept_query) = proposition {
                for (_, attribute) in concept_query.predicate.with().iter() {
                    if let Ok(entity) = attribute.to_uri().parse::<Entity>() {
                        entities.insert(entity);
                    }
                }
            }
        }
        entities
    }

    /// Concept entities referenced by the body's positive `when`
    /// premises. Used by the install-time trigger check to query
    /// each concept's transient marker.
    pub fn when_concept_entities(&self) -> BTreeSet<Entity> {
        let descriptor = self.rule.descriptor();
        let mut entities = BTreeSet::new();
        for proposition in descriptor.when.iter() {
            if let Proposition::Concept(concept_query) = proposition {
                entities.insert(concept_query.predicate.this());
            }
        }
        entities
    }

    /// Look up an effect by its entity URI.
    pub fn by_entity(entity: Entity) -> EffectByEntity {
        EffectByEntity { entity }
    }

    /// Validate the V1 trigger requirement against a branch: at
    /// least one positive `when` premise must read a concept
    /// marked transient.
    ///
    /// This is the install-time check. Construction
    /// ([`Effect::new`], [`Effect::asserting`], etc.) is
    /// permissive; whoever installs an effect into a branch is
    /// responsible for running this check first.
    pub async fn validate<Env: EffectEnv>(
        &self,
        branch: &Branch,
        env: &Env,
    ) -> Result<(), EffectValidationError> {
        let concepts = self.when_concept_entities();
        for entity in concepts {
            let transient = crate::concept::TransientConcept::is_transient(entity)
                .resolve(branch, env)
                .await
                .map_err(|e| EffectValidationError::Query(format!("{e}")))?;
            if transient {
                return Ok(());
            }
        }
        Err(EffectValidationError::MissingTrigger)
    }
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
// Statement impl — write an Effect into a branch transaction.      //
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

/// Build a runtime [`ArtifactsAttribute`] from a domain + local
/// name. The crate's domains are well-formed so `expect` is safe.
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

impl Statement for Effect {
    fn assert(self, update: &mut impl Update) {
        let this = self.this();
        let description = self.descriptor().description.clone();
        let source = self.source();
        let conclusion = self.conclusion();
        let polarity = self.polarity;
        let premises = self.premise_entities();

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
        // Premise attribute index (cardinality-many).
        for premise in premises {
            update.associate(
                meta_attr("dialog.effect", "premise"),
                this.clone(),
                Value::Entity(premise),
            );
        }
        // Optional description, shared with the rest of
        // tonk-schema under `dialog.meta/description`.
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

    fn retract(self, update: &mut impl Update) {
        let this = self.this();
        let description = self.descriptor().description.clone();
        let source = self.source();
        let conclusion = self.conclusion();
        let polarity = self.polarity;
        let premises = self.premise_entities();

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
        for premise in premises {
            update.dissociate(
                meta_attr("dialog.effect", "premise"),
                this.clone(),
                Value::Entity(premise),
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
}

// ---------------------------------------------------------------- //
// Loading effects back from a branch.                              //
// ---------------------------------------------------------------- //

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

/// Builder for [`Effect::by_entity`]. Resolves the
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

/// Look up all effect entities whose `dialog.effect/premise`
/// cardinality-many index contains the given attribute entity.
/// The reverse-index query the reactor's evaluator runs per round
/// to find effects whose body could have been affected by a
/// change.
pub fn effects_by_premise(attribute_entity: Entity) -> EffectsByPremise {
    EffectsByPremise { attribute_entity }
}

/// Builder for [`effects_by_premise`].
pub struct EffectsByPremise {
    attribute_entity: Entity,
}

impl EffectsByPremise {
    /// Resolve every effect entity whose premise index includes
    /// `attribute_entity`.
    pub async fn resolve<Env: EffectEnv>(
        self,
        branch: &Branch,
        env: &Env,
    ) -> Result<Vec<Entity>, EffectLookupError> {
        let claims: Vec<dialog_query::Claim> = branch
            .query()
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the("dialog.effect", "premise"))
                    .of(Term::<Entity>::var("effect"))
                    .is(Term::<Entity>::from(self.attribute_entity.clone())),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| EffectLookupError::Query(format!("premise query failed: {e:?}")))?;
        let mut out: Vec<Entity> = Vec::with_capacity(claims.len());
        for claim in claims {
            out.push(claim.of);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    //! Tests construct effects directly from `InductiveRule` +
    //! `EffectPolarity` and verify the storage shape, entity URI
    //! determinism, and source round-trip. The V1 transient-
    //! trigger check is enforced at install time against a
    //! branch; tests covering that live with the install path.

    use super::*;
    use dialog_query::artifact::{Entity as ArtifactsEntity, Type};
    use dialog_query::attribute::{AttributeDescriptor, Cardinality};
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::formula::Formula;
    use dialog_query::formula::math::Sum;
    use dialog_query::parameters::Parameters;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::the;

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
        DialogPremise::Assert(dialog_query::Proposition::Concept(ConceptQuery {
            terms,
            predicate,
        }))
    }

    /// Helper: rebind a concept premise's named field. Lets
    /// `counter_head()` read its `count` into `?prev`.
    fn rename_field(p: DialogPremise, from: &str, to: &str) -> DialogPremise {
        match p {
            DialogPremise::Assert(dialog_query::Proposition::Concept(mut cq)) => {
                if cq.terms.get(from).is_some() {
                    cq.terms.insert(from.to_string(), Term::var(to));
                }
                DialogPremise::Assert(dialog_query::Proposition::Concept(cq))
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
    fn it_constructs_an_assert_effect() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        assert_eq!(effect.polarity(), EffectPolarity::Assert);
        assert_eq!(effect.conclusion(), counter_head().this());
    }

    #[dialog_common::test]
    fn it_constructs_a_retract_effect() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::retracting(rule);
        assert_eq!(effect.polarity(), EffectPolarity::Retract);
        assert_eq!(effect.conclusion(), counter_head().this());
    }

    #[dialog_common::test]
    fn assert_and_retract_versions_have_distinct_entities() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let asserting = Effect::asserting(rule.clone());
        let retracting = Effect::retracting(rule);
        assert_ne!(asserting.this(), retracting.this());
        assert!(asserting.this().to_string().starts_with("effect:"));
        assert!(retracting.this().to_string().starts_with("effect:"));
    }

    #[dialog_common::test]
    fn effect_entity_is_deterministic() {
        let rule_a = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let rule_b = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect_a = Effect::asserting(rule_a);
        let effect_b = Effect::asserting(rule_b);
        assert_eq!(effect_a.this(), effect_b.this());
    }

    #[dialog_common::test]
    fn it_round_trips_through_source() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let source = effect.source();
        let reloaded = Effect::from_source(&source, effect.polarity()).expect("source round-trips");
        assert_eq!(effect.this(), reloaded.this());
        assert_eq!(effect.polarity(), reloaded.polarity());
    }

    #[dialog_common::test]
    fn it_indexes_premise_attributes() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let premises = effect.premise_entities();
        // Body has two concept premises:
        //   counter (one attribute: counter/count)
        //   increment (one attribute: command/subject)
        // Plus a math/sum formula premise that contributes nothing.
        assert_eq!(premises.len(), 2);
        for (_, attr) in counter_head().with().iter() {
            let uri: Entity = attr.to_uri().parse().expect("valid attribute URI");
            assert!(premises.contains(&uri));
        }
        for (_, attr) in increment_concept().with().iter() {
            let uri: Entity = attr.to_uri().parse().expect("valid attribute URI");
            assert!(premises.contains(&uri));
        }
    }

    #[dialog_common::test]
    fn it_collects_when_concept_entities() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let concepts = effect.when_concept_entities();
        // counter and increment are the two concept premises in
        // `when`. Both contribute their head URIs.
        assert!(concepts.contains(&counter_head().this()));
        assert!(concepts.contains(&increment_concept().this()));
    }

    #[dialog_common::test]
    fn it_emits_expected_facts_when_asserted() {
        use dialog_artifacts::{Changes, Instruction};

        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let this = effect.this();
        let source = effect.source();
        let conclusion = effect.conclusion();
        let premise_set = effect.premise_entities();

        let mut changes = Changes::default();
        effect.assert(&mut changes);

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

        for premise in &premise_set {
            assert!(
                asserted.iter().any(|c| {
                    c.the.to_string() == "dialog.effect/premise"
                        && c.of == this
                        && matches!(&c.is, Value::Entity(e) if *e == *premise)
                }),
                "missing dialog.effect/premise claim for {premise}"
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
        effect.assert(&mut changes);

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
        use crate::concept::TransientConcept;
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
        effect.validate(&branch, &operator).await?;
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

        match effect.validate(&branch, &operator).await {
            Err(EffectValidationError::MissingTrigger) => Ok(()),
            other => panic!("expected MissingTrigger, got {other:?}"),
        }
    }
}
