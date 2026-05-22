//! Storage schema for effects: inductive rules with polarity.
//!
//! This module holds the pure [`Effect`] data type and its
//! storage-shape projections. It depends only on `dialog-*`
//! crates and lives in the `tonk-core` leaf crate. The query
//! and resolution machinery — loading effects back from a
//! branch, the install-time trigger check, the
//! `AnonymousRuleQuery` — lives in `tonk_schema::effect_query`.
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
//! - `dialog.effect/on` — index listing every attribute the
//!   body reads from, encoded as `on:<domain>/<name>` entity
//!   URIs. Cardinality-many; one claim per distinct attribute
//!   in any premise (positive `when` ∪ negative `unless`). The
//!   key form is recoverable from a runtime `Changes`
//!   instruction's `Attribute(domain/name)` without any schema
//!   lookup — the reactor's loop builds `on:<that string>`
//!   directly. Dialog's `Uri::key_bytes()` projection packs the
//!   prefix into 32 bytes and hashes any overflow, so short
//!   attribute names get full sort-locality and long ones still
//!   collide cleanly.
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
use dialog_artifacts::Entity;
use dialog_common::Blake3Hash;
use dialog_query::{Attribute, InductiveRule, InductiveRuleDescriptor, Proposition};
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

/// An attribute the effect's body reads from, encoded as the
/// entity URI `on:<domain>/<name>` so the reactor can build the
/// key directly from a [`Changes`](dialog_artifacts::Changes)
/// instruction's `Attribute` without resolving the full
/// [`AttributeDescriptor`](dialog_query::AttributeDescriptor).
/// Cardinality-many: one claim per distinct attribute the body
/// reads (positive `when` ∪ negative `unless`).
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cardinality(many)]
#[domain("dialog.effect")]
pub struct On(pub Entity);

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

    /// The set of `on:<domain>/<name>` entity URIs for every
    /// attribute the body reads from. For each
    /// `Proposition::Concept` premise in `when` or `unless`,
    /// every attribute referenced by the premise's predicate
    /// contributes one URI. Values of the `dialog.effect/on`
    /// claims.
    ///
    /// The URI form is recoverable at runtime from a `Changes`
    /// instruction's `Attribute` alone: `on:` + the runtime
    /// `domain/name`. No schema lookup needed to compute the
    /// reverse-index key.
    ///
    /// Attribute-direct premises (`Proposition::Attribute`) are
    /// not currently included; they read individual EAV triples
    /// directly, which the yaml authoring surface doesn't expose.
    ///
    /// Formula premises (`math/sum`, `==`, etc.) contribute
    /// nothing.
    pub fn on_entities(&self) -> BTreeSet<Entity> {
        let descriptor = self.rule.descriptor();
        let mut entities = BTreeSet::new();
        for proposition in descriptor.when.iter().chain(descriptor.unless.iter()) {
            if let Proposition::Concept(concept_query) = proposition {
                for (_, attribute) in concept_query.predicate.with().iter() {
                    let the = attribute.the();
                    let uri = format!("on:{}/{}", the.domain(), the.name());
                    if let Ok(entity) = uri.parse::<Entity>() {
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
}

#[cfg(test)]
mod tests {
    //! Tests construct effects directly from `InductiveRule` +
    //! `EffectPolarity` and verify the storage shape, entity URI
    //! determinism, and source round-trip. The V1 transient-
    //! trigger check is enforced at install time against a
    //! branch; tests covering that live with the install path
    //! (`tonk_schema::effect_query`).

    use super::*;
    use dialog_artifacts::Value;
    use dialog_query::artifact::{Entity as ArtifactsEntity, Type};
    use dialog_query::attribute::{AttributeDescriptor, Cardinality};
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::formula::Formula;
    use dialog_query::formula::math::Sum;
    use dialog_query::parameters::Parameters;
    use dialog_query::premise::Premise as DialogPremise;
    use dialog_query::{Term, the};

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
    fn it_indexes_on_attributes() {
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let attributes = effect.on_entities();
        // Body has two concept premises:
        //   counter (one attribute: counter/count)
        //   increment (one attribute: command/subject)
        // Plus a math/sum formula premise that contributes nothing.
        assert_eq!(attributes.len(), 2);
        for (_, attr) in counter_head().with().iter() {
            let the = attr.the();
            let uri: Entity = format!("on:{}/{}", the.domain(), the.name())
                .parse()
                .expect("valid on: URI");
            assert!(attributes.contains(&uri));
        }
        for (_, attr) in increment_concept().with().iter() {
            let the = attr.the();
            let uri: Entity = format!("on:{}/{}", the.domain(), the.name())
                .parse()
                .expect("valid on: URI");
            assert!(attributes.contains(&uri));
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
}
