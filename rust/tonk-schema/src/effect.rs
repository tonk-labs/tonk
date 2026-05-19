//! Storage schema for inductive rules (a.k.a. effects).
//!
//! An effect is an [`InductiveRule`](dialog_query::InductiveRule)
//! reified as facts on a branch so it replicates and is queryable
//! like any other concept. The reactor's evaluator loads stored
//! effects on each commit, fires whatever matches, and produces
//! head facts as part of the transaction.
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
//! - `dialog.effect/assert` — index pointing at the head concept
//!   entity. One claim per effect (the head's
//!   [`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this)).
//! - `dialog.effect/premise` — index listing every concept entity
//!   referenced by any premise in the body (positive `when` ∪
//!   negative `unless`). Cardinality-many; one claim per distinct
//!   concept the body reads. The two clause kinds collapse into a
//!   single attribute so the reverse-index query stays a single
//!   one-hop lookup against `dialog.effect/premise` — splitting
//!   them would force the evaluator to union two queries on every
//!   round.
//! - `dialog.effect/description` — optional human-readable
//!   description.
//!
//! The index attributes are pure projections of the source. They
//! exist so the evaluator can answer "which effects could be
//! affected by a change to concept X?" with a single one-hop
//! query against `dialog.effect/premise`, without deserializing
//! every effect's body. Whether a premise is positive or negative
//! lives inside the source JSON; the evaluator deserializes once
//! a rule has been selected as a candidate.
//!
//! # The `effect:system` sentinel
//!
//! Effects observe local commands by reading premises with
//! `this: effect:system`. Facts asserted on this sentinel entity
//! are commit-scoped: the reactor's evaluator reads them during
//! the firing loop and strips them before the durable delta is
//! written. See `plan/effects.md` for the full design.

// `#[derive(Attribute)]` expands to helper items without doc
// comments; suppress the crate-level `missing_docs` lint here.
#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::sync::LazyLock;

use base58::ToBase58;
use dialog_artifacts::{Entity, Value};
use dialog_common::Blake3Hash;
use dialog_query::{Attribute, InductiveRule, InductiveRuleDescriptor, Proposition, Term};
use thiserror::Error;

/// Sentinel entity URI for the ambient command bus. Facts
/// asserted on this entity are commit-scoped and never persist —
/// the reactor's effect evaluator reads them, fires whatever
/// matches, and drops them before the durable delta is written.
///
/// Mental model: stdin or the deprecated `window.event` — an
/// ambient stream of commands handlers read but no one stores.
pub const EFFECT_SYSTEM_URI: &str = "did:key:zEffectSystem";

/// `effect:system` as a parsed [`Entity`]. The reactor compares
/// premise terms against this to identify command-shaped
/// triggers (premises with `terms["this"] == EFFECT_SYSTEM`).
pub static EFFECT_SYSTEM: LazyLock<Entity> = LazyLock::new(|| {
    EFFECT_SYSTEM_URI
        .parse()
        .expect("EFFECT_SYSTEM_URI is a valid entity URI")
});

/// The canonical JSON of an effect's [`InductiveRuleDescriptor`].
/// Source of truth — every other `dialog.effect/*` attribute is a
/// derived index that the reactor recomputes from this claim.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.effect")]
pub struct Source(pub String);

/// The concept entity this effect asserts when its body matches.
/// Equal to the descriptor's head's
/// [`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this).
/// One claim per effect.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("dialog.effect")]
pub struct Assert(pub Entity);

/// A concept entity referenced by some premise in the effect's
/// body — either a positive `when` premise or a negative `unless`
/// premise. Cardinality-many: one claim per distinct concept the
/// body reads.
///
/// The reactor's evaluator queries this index per round to decide
/// which effects could be re-fired by the dirty set of concepts.
/// Formula premises (`math/sum`, `==`, etc.) don't appear here —
/// they compute from bound variables rather than reading concept
/// state, so a change to a formula's "predicate" can never
/// re-trigger anything.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cardinality(many)]
#[domain("dialog.effect")]
pub struct Premise(pub Entity);

// ---------------------------------------------------------------- //
// Effect wrapper                                                   //
// ---------------------------------------------------------------- //

/// Reasons an [`InductiveRule`] cannot be wrapped as an [`Effect`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EffectError {
    /// The rule's body does not contain a positive premise reading
    /// from `effect:system`. V1 effects must include such a
    /// premise so the runtime knows the rule only fires on local
    /// commands and can skip pull-time re-evaluation.
    #[error(
        "inductive rule has no positive `effect:system` trigger — \
        V1 effects must include `where: {{ this: effect:system, ... }}` \
        on at least one `when` premise"
    )]
    MissingTrigger,

    /// Could not deserialize the effect's `dialog.effect/source`
    /// claim as an [`InductiveRuleDescriptor`]. The text was not
    /// valid JSON, or the JSON did not match the descriptor's
    /// shape.
    #[error("failed to parse stored effect source: {0}")]
    Deserialize(String),
}

/// An [`InductiveRule`] that satisfies the V1 effect invariant:
/// at least one positive `when` premise reads from `effect:system`.
/// Constructed via [`Effect::from_rule`]; cannot be built bypassing
/// the check.
///
/// Treat this like a newtype: the underlying [`InductiveRule`] is
/// available via [`Effect::rule`] / [`Effect::into_rule`], but the
/// only way to *make* one is by going through [`Effect::from_rule`]
/// or [`Effect::from_source`] so the V1 invariant is upheld.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    rule: InductiveRule,
}

impl Effect {
    /// Wrap a compiled [`InductiveRule`], rejecting it if it does
    /// not include a positive `effect:system` trigger.
    pub fn from_rule(rule: InductiveRule) -> Result<Self, EffectError> {
        if !has_effect_system_trigger(&rule) {
            return Err(EffectError::MissingTrigger);
        }
        Ok(Effect { rule })
    }

    /// Parse the canonical JSON form of an
    /// [`InductiveRuleDescriptor`] and wrap as an [`Effect`].
    /// Used by the reactor when loading effects back from a
    /// branch's `dialog.effect/source` claims.
    pub fn from_source(source: &str) -> Result<Self, EffectError> {
        let rule: InductiveRule =
            serde_json::from_str(source).map_err(|e| EffectError::Deserialize(e.to_string()))?;
        Self::from_rule(rule)
    }

    /// Borrow the wrapped rule.
    pub fn rule(&self) -> &InductiveRule {
        &self.rule
    }

    /// Unwrap into the underlying rule.
    pub fn into_rule(self) -> InductiveRule {
        self.rule
    }

    /// Reconstruct the serializable descriptor for this effect.
    pub fn descriptor(&self) -> InductiveRuleDescriptor {
        self.rule.descriptor()
    }

    /// The effect's content-addressed entity URI. Two peers that
    /// independently construct the same effect (same head, same
    /// body) converge on the same entity.
    ///
    /// Derived from the blake3 hash of the descriptor's dag-cbor
    /// encoding, with an `effect:` URI scheme — mirroring how
    /// [`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this)
    /// produces `concept:<hash>` URIs.
    pub fn this(&self) -> Entity {
        let descriptor = self.rule.descriptor();
        let bytes = serde_ipld_dagcbor::to_vec(&descriptor)
            .expect("dag-cbor encoding of InductiveRuleDescriptor should not fail");
        let hash = Blake3Hash::hash(&bytes);
        let encoded = hash.as_bytes().as_ref().to_base58();
        format!("effect:{encoded}")
            .parse()
            .expect("effect:<base58> is a valid entity URI")
    }

    /// The canonical JSON form of this effect's descriptor — the
    /// value of the `dialog.effect/source` claim.
    pub fn source(&self) -> String {
        serde_json::to_string(&self.rule.descriptor())
            .expect("InductiveRuleDescriptor always serializes to JSON")
    }

    /// The head concept entity — the value of the
    /// `dialog.effect/assert` claim. Renamed away from
    /// `assert()` so it doesn't collide with the
    /// [`Statement::assert`] trait method.
    pub fn head(&self) -> Entity {
        self.rule.conclusion().this()
    }

    /// The set of attribute entities the body reads from. For
    /// each `Proposition::Concept` premise in `when` or `unless`,
    /// every attribute referenced by the premise's predicate
    /// contributes its `the:<hash>` URI to the set. Values of
    /// the `dialog.effect/premise` claims.
    ///
    /// The index is attribute-keyed (not concept-keyed) so it
    /// invalidates correctly under concept-lens-sharing: if two
    /// concepts share an attribute URI, a change to that
    /// attribute affects every effect that reads it via any
    /// concept lens.
    ///
    /// Attribute-direct premises (`Proposition::Attribute`) are
    /// *not* currently included; they read individual EAV
    /// triples directly, which the yaml authoring surface
    /// doesn't expose anyway.
    ///
    /// Formula premises (`math/sum`, `==`, etc.) contribute
    /// nothing: they compute from bound variables rather than
    /// reading concept state.
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
}

/// True if any positive `when` premise is a `Proposition::Concept`
/// whose `this` term binds to `effect:system`. The V1 effect
/// invariant requires at least one such premise; the runtime
/// uses this property to skip pull-time re-evaluation.
fn has_effect_system_trigger(rule: &InductiveRule) -> bool {
    let descriptor = rule.descriptor();
    descriptor.when.iter().any(|premise| match premise {
        Proposition::Concept(concept_query) => match concept_query.terms.get("this") {
            Some(Term::Constant(Value::Entity(e))) => e == &*EFFECT_SYSTEM,
            _ => false,
        },
        Proposition::Attribute(_) | Proposition::Formula(_) | Proposition::Constraint(_) => false,
    })
}

// ---------------------------------------------------------------- //
// Statement impl — write an Effect into a branch transaction.      //
// ---------------------------------------------------------------- //

use dialog_artifacts::{Attribute as ArtifactsAttribute, Update};
use dialog_query::Statement;

/// The well-known marker entity asserted as the value of
/// `dialog.meta/effect` on every effect entity, mirroring how
/// concept entities carry `(?this, dialog.meta/concept,
/// db:concept)`. Lets `"all effects on this branch"` queries
/// start from a selectable triple.
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
        let assert = self.head();
        let premises = self.premise_entities();

        // Marker claim — `(?this, dialog.meta/effect, db:effect)`.
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
        // Head-concept index claim.
        update.associate_unique(
            meta_attr("dialog.effect", "assert"),
            this.clone(),
            Value::Entity(assert),
        );
        // Premise-concept index claims (cardinality-many).
        for premise in premises {
            update.associate(
                meta_attr("dialog.effect", "premise"),
                this.clone(),
                Value::Entity(premise),
            );
        }
        // Optional human-readable description, written under the
        // shared `dialog.meta/description` attribute the rest of
        // tonk-schema also uses.
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
        let assert = self.head();
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
            meta_attr("dialog.effect", "assert"),
            this.clone(),
            Value::Entity(assert),
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

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::memory::Resolve;
use dialog_query::Output as _;
use dialog_repository::{Branch, RemoteSite};
use thiserror::Error as ThisError;

/// Trait alias gathering the capability bounds every effect
/// resolver needs. Mirrors `concept::QueryEnv`.
pub trait EffectEnv:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
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
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Failures specific to loading an effect from a branch.
#[derive(Debug, ThisError)]
pub enum EffectLookupError {
    /// The branch query infrastructure returned an error.
    #[error("effect lookup query failed: {0}")]
    Query(String),
    /// The effect entity was found but its
    /// `dialog.effect/source` claim couldn't be parsed back into
    /// a valid [`Effect`].
    #[error(transparent)]
    Effect(#[from] EffectError),
}

/// Parse a `domain/name` pair as a typed `The`. The crate's
/// effect attributes are well-formed at compile time so `expect`
/// is appropriate.
fn the(domain: &str, name: &str) -> dialog_query::attribute::The {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog.effect attribute name is well-formed")
}

impl Effect {
    /// Look up an effect by its entity URI.
    pub fn by_entity(entity: Entity) -> EffectByEntity {
        EffectByEntity { entity }
    }
}

/// Builder for [`Effect::by_entity`]. Resolves the
/// `dialog.effect/source` claim and rehydrates the effect.
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
        let claims: Vec<dialog_query::Claim> = branch
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

        let Some(claim) = claims.into_iter().next() else {
            return Ok(None);
        };
        let source = match claim.is {
            Value::String(s) => s,
            other => {
                return Err(EffectLookupError::Query(format!(
                    "dialog.effect/source claim was not a string: {other:?}"
                )));
            }
        };
        let effect = Effect::from_source(&source)?;
        Ok(Some(effect))
    }
}

/// Look up all effects whose `dialog.effect/premise` cardinality-many
/// index contains the given concept entity. This is the reverse-index
/// query the reactor's evaluator will use per round to find effects
/// whose body could have been affected by a change to that concept.
pub fn effects_by_premise(concept_entity: Entity) -> EffectsByPremise {
    EffectsByPremise { concept_entity }
}

/// Builder for [`effects_by_premise`].
pub struct EffectsByPremise {
    concept_entity: Entity,
}

impl EffectsByPremise {
    /// Resolve every effect entity whose premise index includes
    /// `concept_entity`.
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
                    .is(Term::<Entity>::from(self.concept_entity.clone())),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| EffectLookupError::Query(format!("premise query failed: {e:?}")))?;
        let mut out: Vec<Entity> = Vec::with_capacity(claims.len());
        for claim in claims {
            // The query bound `of` to a variable; reading it back
            // from the claim gives us the effect entity for each
            // matched premise edge.
            out.push(claim.of);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    //! Tests model the increment-counter effect: an `increment`
    //! command on `effect:system` targeting some counter triggers
    //! a rule that reads the counter's current count and asserts
    //! a new counter row with `count + 1`. This is the canonical
    //! V1 effect shape — the trigger lives on `effect:system`,
    //! so it never replicates and the runtime can skip pull-time
    //! re-evaluation.

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

    /// `increment` command concept — has a `subject` field
    /// naming the counter to bump.
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

    /// Build a `ConceptQuery` premise binding `this` to a
    /// constant entity and other fields to named variables.
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

    fn increment_with_trigger() -> Vec<DialogPremise> {
        let mut sum_terms = Parameters::new();
        sum_terms.insert("of".to_string(), Term::var("prev"));
        sum_terms.insert("with".to_string(), Term::constant(1u64));
        sum_terms.insert("is".to_string(), Term::var("count"));
        vec![
            // Read the counter's current count into ?prev.
            concept_premise(counter_head(), Term::<ArtifactsEntity>::var("this"))
                .renaming_field("count", "prev"),
            // Trigger: an `increment` command on effect:system.
            concept_premise(
                increment_concept(),
                Term::Constant(Value::Entity(EFFECT_SYSTEM.clone())),
            ),
            // ?count = ?prev + 1.
            Sum::apply(sum_terms)
                .expect("Sum::apply should succeed")
                .into(),
        ]
    }

    fn increment_without_trigger() -> Vec<DialogPremise> {
        // Drops the effect:system premise. Reads counter directly,
        // no command — no trigger.
        let mut sum_terms = Parameters::new();
        sum_terms.insert("of".to_string(), Term::var("prev"));
        sum_terms.insert("with".to_string(), Term::constant(1u64));
        sum_terms.insert("is".to_string(), Term::var("count"));
        vec![
            concept_premise(counter_head(), Term::<ArtifactsEntity>::var("this"))
                .renaming_field("count", "prev"),
            Sum::apply(sum_terms)
                .expect("Sum::apply should succeed")
                .into(),
        ]
    }

    /// Helper trait: rebind a concept premise's named field to a
    /// different variable name. Lets `counter_head()` read
    /// `count` into `?prev` without inventing a new descriptor.
    trait RenameField {
        fn renaming_field(self, from: &str, to: &str) -> Self;
    }

    impl RenameField for DialogPremise {
        fn renaming_field(self, from: &str, to: &str) -> Self {
            match self {
                DialogPremise::Assert(dialog_query::Proposition::Concept(mut cq)) => {
                    if cq.terms.get(from).is_some() {
                        cq.terms.insert(from.to_string(), Term::var(to));
                    }
                    DialogPremise::Assert(dialog_query::Proposition::Concept(cq))
                }
                other => other,
            }
        }
    }

    #[dialog_common::test]
    fn it_wraps_rule_with_effect_system_trigger() {
        let rule = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let effect = Effect::from_rule(rule).expect("rule has trigger");
        assert_eq!(effect.head(), counter_head().this());
    }

    #[dialog_common::test]
    fn it_rejects_rule_without_trigger() {
        let rule = InductiveRule::new(counter_head(), increment_without_trigger())
            .expect("rule should compile");
        assert_eq!(
            Effect::from_rule(rule).unwrap_err(),
            EffectError::MissingTrigger
        );
    }

    #[dialog_common::test]
    fn it_round_trips_through_source() {
        let rule = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let effect = Effect::from_rule(rule).expect("rule has trigger");
        let source = effect.source();
        let reloaded = Effect::from_source(&source).expect("source round-trips");
        assert_eq!(effect.this(), reloaded.this());
    }

    #[dialog_common::test]
    fn it_indexes_premise_attributes() {
        let rule = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let effect = Effect::from_rule(rule).expect("rule has trigger");
        let premises = effect.premise_entities();
        // Body has two concept premises:
        //   - counter (one attribute: counter/count)
        //   - increment (one attribute: command/subject)
        // The index is attribute-keyed, so we expect one entity
        // per attribute referenced. The math/sum formula premise
        // contributes nothing.
        assert_eq!(premises.len(), 2);
        for (_, attr) in counter_head().with().iter() {
            let uri: Entity = attr.to_uri().parse().expect("valid attribute URI");
            assert!(
                premises.contains(&uri),
                "missing counter attribute {uri} in premise index"
            );
        }
        for (_, attr) in increment_concept().with().iter() {
            let uri: Entity = attr.to_uri().parse().expect("valid attribute URI");
            assert!(
                premises.contains(&uri),
                "missing increment attribute {uri} in premise index"
            );
        }
    }

    #[dialog_common::test]
    fn it_emits_expected_facts_when_asserted() {
        use dialog_artifacts::{Changes, Instruction};
        use dialog_query::Statement;

        let rule = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let effect = Effect::from_rule(rule).expect("rule has trigger");
        let this = effect.this();
        let source = effect.source();
        let head = effect.head();
        let premise_set = effect.premise_entities();

        let mut changes = Changes::default();
        effect.assert(&mut changes);

        // Drain the changeset into a flat list of Asserts (we
        // emit no retracts or replaces here). `associate_unique`
        // surfaces as `Instruction::Replace`; `associate` as
        // `Instruction::Assert`. Both count as "asserted facts"
        // for this test.
        let asserted: Vec<_> = changes
            .into_instructions()
            .into_iter()
            .filter_map(|inst| match inst {
                Instruction::Assert(a) | Instruction::Replace(a) => Some(a),
                Instruction::Retract(_) => None,
            })
            .collect();

        let marker_entity = effect_marker_entity();
        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.meta/effect"
                    && c.of == this
                    && matches!(&c.is, Value::Entity(e) if *e == marker_entity)
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
                c.the.to_string() == "dialog.effect/assert"
                    && c.of == this
                    && matches!(&c.is, Value::Entity(e) if *e == head)
            }),
            "missing dialog.effect/assert claim"
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
    fn effect_entity_is_deterministic() {
        let rule_a = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let rule_b = InductiveRule::new(counter_head(), increment_with_trigger())
            .expect("rule should compile");
        let effect_a = Effect::from_rule(rule_a).unwrap();
        let effect_b = Effect::from_rule(rule_b).unwrap();
        assert_eq!(effect_a.this(), effect_b.this());
        assert!(effect_a.this().to_string().starts_with("effect:"));
    }
}
