//! Rule — the rule-of-rules' `Statement` adapter, parallel to
//! [`AnonymousConcept`](crate::concept::AnonymousConcept).
//!
//! `AnonymousConcept` packages a [`ConceptDescriptor`] with the
//! storage shape (`dialog.meta/concept` marker plus
//! `dialog.concept.with/*` per-field claims) so a `concept!:` notation
//! can flow through dialog's `Statement` trait. [`Rule`] plays the
//! same role for installed inductive rules: it packages an
//! [`Effect`] with the storage shape (`dialog.meta/effect` marker
//! plus `dialog.effect/{source,conclusion,polarity,on}` claims) so
//! `rule!:` notation flows through the same trait.
//!
//! # Two construction paths — symmetric assert, careful retract
//!
//! Asserting a new rule is purely synthetic: hash the rule body,
//! serialise the source, emit claims. Retracting needs to *match*
//! what was previously stored. The risk lives in
//! `dialog.effect/source`: it carries the JSON-encoded
//! [`InductiveRuleDescriptor`], and serialisation is not guaranteed
//! to round-trip byte-for-byte (premise order, optional fields,
//! whitespace…). If a fresh [`Effect::source`] disagrees with the
//! stored string by one byte, the dissociate misses, the source
//! claim survives the retraction, and the rule re-installs itself
//! the next time anyone reads from the branch.
//!
//! So we offer two constructors:
//!
//! - [`Rule::asserting`] — wrap a freshly-built [`Effect`] for
//!   immediate `assert`. The carried source string is whatever the
//!   effect would serialise to right now; that *is* the stored
//!   string a moment later, so the symmetry holds for an
//!   assert+retract pair issued back-to-back in the same process.
//! - [`Rule::retracting`] — async builder that reads the stored
//!   `dialog.effect/source` and `dialog.effect/polarity` claims off
//!   a branch and bakes them into a [`Rule`] whose `Statement`
//!   impl dissociates using *those exact* bytes. This is the path
//!   for `rule!: this: <entity> ..: _` deletions where the rule
//!   may have been written by a different process or version.
//!
//! Both paths produce the same `Rule` shape. Once the resolver has
//! run, the resulting value lives inside the dialog `Statement`
//! protocol's synchronous boundary.
//!
//! # Why `Rule` over `EffectStatement`
//!
//! Naming follows the AnonymousConcept analogy: the type is the
//! rule-as-data side of the install path, not "a wrapper around
//! [`Effect`]". `Effect` remains the pure data type in
//! [`tonk_core::effect`]; `Rule` is the schema-aware view that
//! plugs into the branch's mutation surface.
//!
//! [`ConceptDescriptor`]: dialog_query::ConceptDescriptor
//! [`Effect`]: tonk_core::effect::Effect
//! [`InductiveRuleDescriptor`]: dialog_query::InductiveRuleDescriptor

pub use dialog_query::{
    DeductiveRule, DeductiveRuleDescriptor, InductiveRule, InductiveRuleDescriptor,
};

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_query::{Output as _, Term};
use thiserror::Error;

use tonk_core::effect::{Effect, EffectPolarity};

use crate::concept::QueryEnv;
use crate::query_source::Source;

/// A rule installed on a branch — pairs an [`Effect`] with the
/// exact `dialog.effect/source` string and polarity tag used to
/// store it (or to retract it).
///
/// See the [module docs](self) for why this carries the source
/// string as a separate field rather than re-deriving it from the
/// [`Effect`] inside the `Statement` impl.
///
/// Construct via [`Rule::asserting`] for content-addressed installs,
/// [`Rule::asserting_at`] for installs at a caller-chosen entity, or
/// [`Rule::retracting`] for deletions.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The installed effect — head, body, polarity.
    pub effect: Effect,
    /// The entity URI the `dialog.effect/*` claims are written
    /// against. Defaults to [`Effect::this`] (content-derived) for
    /// [`Rule::asserting`]; callers can override via
    /// [`Rule::asserting_at`] to install at a stable, user-chosen
    /// entity. On the retract path this is the entity whose stored
    /// claims [`Retracting::resolve`] read, so the dissociate hits
    /// the same EAVs that were written.
    this: Entity,
    /// The exact source string carried on
    /// `dialog.effect/source`. Synthesised by
    /// [`Effect::source`](tonk_core::effect::Effect::source) on
    /// the assert path; read off the branch on the retract path.
    source: String,
    /// The polarity string carried on
    /// `dialog.effect/polarity`. Stored as a separate field rather
    /// than re-derived so the retract path is symmetric with
    /// assert.
    polarity: EffectPolarity,
}

impl Rule {
    /// Wrap a freshly-built [`Effect`] for an `assert` mutation
    /// against the effect's content-derived entity ([`Effect::this`]).
    ///
    /// The carried `source` string is whatever the effect
    /// serialises to right now. That's exactly what
    /// [`Statement::assert`](dialog_artifacts::Statement::assert)
    /// will write under the `dialog.effect/source` claim, so the
    /// pair is consistent by construction.
    pub fn asserting(effect: Effect) -> Self {
        let this = effect.this();
        Self::asserting_at(effect, this)
    }

    /// Wrap a freshly-built [`Effect`] for an `assert` mutation
    /// against a caller-chosen entity. Lets the user install a rule
    /// at a stable URI (`rule!: this: <entity>, ..`) instead of the
    /// content-derived hash, so future retracts can name the entity
    /// directly.
    pub fn asserting_at(effect: Effect, entity: Entity) -> Self {
        let source = effect.source();
        let polarity = effect.polarity();
        Self {
            effect,
            this: entity,
            source,
            polarity,
        }
    }

    /// Begin building a [`Rule`] for a `retract` mutation. The
    /// returned [`Retracting`] resolves against a branch to read
    /// the stored `dialog.effect/source` + `dialog.effect/polarity`
    /// claims and bake those exact bytes into the resulting `Rule`,
    /// so the dissociate matches byte-for-byte and the source claim
    /// is actually removed.
    pub fn retracting(entity: Entity) -> Retracting {
        Retracting { entity }
    }

    /// The entity URI the `dialog.effect/*` claims live under.
    pub fn this(&self) -> Entity {
        self.this.clone()
    }

    /// The exact `dialog.effect/source` string (JSON-encoded
    /// [`InductiveRuleDescriptor`]). Paired with [`Rule::polarity`]
    /// this round-trips through
    /// [`Effect::from_source`](tonk_core::effect::Effect::from_source),
    /// so a rule can be embedded as `(source, polarity, this)` and
    /// rebuilt at runtime — the carrier the `claim!` macro emits
    /// alongside the document's concept claims.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The polarity tag (`assert` / `retract`) carried on
    /// `dialog.effect/polarity`.
    pub fn polarity(&self) -> EffectPolarity {
        self.polarity
    }
}

impl dialog_artifacts::Statement for Rule {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        let this = self.this.clone();
        let description = self.effect.descriptor().description.clone();
        let conclusion = self.effect.conclusion();
        let attributes = self.effect.on_entities();

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
            Value::String(self.source),
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
            Value::String(self.polarity.as_str().to_owned()),
        );
        // Per-attribute reverse index (cardinality-many).
        for attribute in attributes {
            update.associate(
                meta_attr("dialog.effect", "on"),
                this.clone(),
                Value::Entity(attribute),
            );
        }
        // Optional description.
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

    fn retract(self, update: &mut impl dialog_artifacts::Update) {
        let this = self.this.clone();
        let description = self.effect.descriptor().description.clone();
        let conclusion = self.effect.conclusion();
        let attributes = self.effect.on_entities();

        update.dissociate(
            meta_attr("dialog.meta", "effect"),
            this.clone(),
            Value::Entity(effect_marker_entity()),
        );
        update.dissociate(
            meta_attr("dialog.effect", "source"),
            this.clone(),
            Value::String(self.source),
        );
        update.dissociate(
            meta_attr("dialog.effect", "conclusion"),
            this.clone(),
            Value::Entity(conclusion),
        );
        update.dissociate(
            meta_attr("dialog.effect", "polarity"),
            this.clone(),
            Value::String(self.polarity.as_str().to_owned()),
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
}

/// Builder for [`Rule::retracting`]. Reads the stored
/// `dialog.effect/source` + `dialog.effect/polarity` claims off a
/// branch so the resulting [`Rule`]'s `retract` dissociates the
/// stored bytes verbatim. See the [module docs](self) for why this
/// extra round-trip matters.
#[derive(Debug, Clone)]
pub struct Retracting {
    entity: Entity,
}

impl Retracting {
    /// Resolve against a branch. Returns `None` when the entity
    /// has no `dialog.effect/source` claim (no such rule installed).
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Rule>, RuleResolveError> {
        let source_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(effect_attr("source"))
                    .of(Term::<Entity>::from(self.entity.clone()))
                    .is(Term::<String>::var("__source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| RuleResolveError::Query(format!("source lookup failed: {e:?}")))?;
        let Some(source_claim) = source_claims.into_iter().next() else {
            return Ok(None);
        };
        let Value::String(source_string) = source_claim.is else {
            return Err(RuleResolveError::Storage(
                "dialog.effect/source claim was not a string".to_owned(),
            ));
        };

        let polarity_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(effect_attr("polarity"))
                    .of(Term::<Entity>::from(self.entity.clone()))
                    .is(Term::<String>::var("__polarity")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| RuleResolveError::Query(format!("polarity lookup failed: {e:?}")))?;
        let Some(polarity_claim) = polarity_claims.into_iter().next() else {
            return Err(RuleResolveError::Storage(
                "missing dialog.effect/polarity claim".to_owned(),
            ));
        };
        let Value::String(polarity_string) = polarity_claim.is else {
            return Err(RuleResolveError::Storage(
                "dialog.effect/polarity claim was not a string".to_owned(),
            ));
        };
        let polarity = EffectPolarity::parse(&polarity_string).ok_or_else(|| {
            RuleResolveError::Storage(format!("invalid polarity {polarity_string:?}"))
        })?;

        // Rehydrate the Effect from the stored source so on_entities,
        // conclusion, descriptor() etc. line up with what was
        // written. The carried `source` field stays the *exact*
        // stored bytes — Effect::from_source may parse-and-rebuild
        // them via serde, but we don't trust that rebuild for the
        // dissociate.
        let effect = Effect::from_source(&source_string, polarity)
            .map_err(|e| RuleResolveError::Storage(format!("effect rehydrate failed: {e}")))?;

        Ok(Some(Rule {
            effect,
            // The stored claims live at `self.entity` regardless of
            // what `effect.this()` would hash to from the rehydrated
            // descriptor. Carry the resolved entity through so the
            // dissociate targets the same EAVs that were written
            // (including content-addressed installs where the two
            // happen to coincide).
            this: self.entity,
            source: source_string,
            polarity,
        }))
    }
}

/// Errors that can happen while resolving an installed rule for
/// retraction. Either the branch query plumbing failed, or the
/// stored facts are malformed.
#[derive(Debug, Error)]
pub enum RuleResolveError {
    /// The branch query infrastructure returned an error.
    #[error("rule resolve query failed: {0}")]
    Query(String),
    /// A stored claim had the wrong shape — wrong value kind,
    /// missing required claim, unrecognised polarity, malformed
    /// source JSON.
    #[error("rule storage shape: {0}")]
    Storage(String),
}

/// Build a runtime [`ArtifactsAttribute`] from a domain + local
/// name. Mirrors the helper used by [`AnonymousConcept`]'s
/// storage emitter so the two paths look the same at the
/// claim-emission level.
///
/// [`AnonymousConcept`]: crate::concept::AnonymousConcept
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

/// Build the typed [`dialog_query::attribute::The`] selector for a
/// `dialog.effect/<name>` attribute. The retract resolver needs the
/// typed form when constructing an [`AttributeQuery`].
fn effect_attr(name: &str) -> dialog_query::attribute::The {
    format!("dialog.effect/{name}")
        .parse()
        .expect("dialog.effect/<name> is a valid attribute URI")
}

/// Marker entity asserted as the value of `dialog.meta/effect` on
/// every effect entity. Same role as `db:concept` for concepts:
/// gives "all effects on this branch" queries a known triple to
/// start from.
fn effect_marker_entity() -> Entity {
    "db:effect"
        .parse()
        .expect("`db:effect` is a valid entity URI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::Statement as ArtifactsStatement;
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

    use tonk_core::effect::EFFECT_SYSTEM;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// `counter` concept with a single `count` field — the same
    /// shape `effect_query::tests` and `rule_query::tests` use, so
    /// rule fixtures stay coherent across the three test modules.
    fn counter_head() -> ConceptDescriptor {
        ConceptDescriptor::try_from(vec![(
            "count",
            AttributeDescriptor::new(
                the!("counter/count"),
                "",
                Cardinality::One,
                Some(Type::UnsignedInt),
            ),
        )])
        .unwrap()
    }

    /// `increment` command concept used by [`increment_body`].
    fn increment_concept() -> ConceptDescriptor {
        ConceptDescriptor::try_from(vec![(
            "subject",
            AttributeDescriptor::new(
                the!("command/subject"),
                "",
                Cardinality::One,
                Some(Type::Entity),
            ),
        )])
        .unwrap()
    }

    /// Concept premise binding `this` and each field to a variable.
    fn concept_premise(predicate: ConceptDescriptor, this: Term<ArtifactsEntity>) -> DialogPremise {
        let mut terms = Parameters::new();
        terms.insert("this".to_string(), this.into());
        for field in predicate.with().iter().map(|(name, _)| name.to_string()) {
            terms.insert(field.clone(), Term::var(&field));
        }
        DialogPremise::Assert(Proposition::Concept(ConceptQuery { terms, predicate }))
    }

    /// Rebind a concept premise's named field. Lets `counter_head()`
    /// read its `count` into `?prev`.
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

    /// Install an effect, then retract it by resolving its
    /// [`Rule::retracting`] handle off the branch and committing the
    /// dissociates. After the retract commit the rule should *not*
    /// surface in a `dialog.effect/source` query — proving the
    /// dissociate matched the stored bytes verbatim. This pins the
    /// regression that bit
    /// commit `fd1cea8f` (reverted at `2abb4b2f`): the previous
    /// retract path re-serialised the source on the fly, so a single
    /// byte of drift between `Effect::source()` invocations would
    /// leave the source claim live and the rule re-installed itself.
    #[dialog_common::test]
    async fn it_retracts_by_reading_the_stored_source_string() -> anyhow::Result<()> {
        use crate::query_source::Source;
        use dialog_query::{AttributeQuery, Output as _};
        use dialog_repository::helpers::{test_operator_with_profile, test_repo};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Install the rule.
        let inductive =
            InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(inductive);
        let entity = effect.this();
        branch
            .transaction()
            .assert(Rule::asserting(effect.clone()))
            .commit()
            .perform(&operator)
            .await?;

        // Sanity: the source claim is there.
        let source_query = AttributeQuery::from(
            Term::<dialog_query::attribute::The>::from(effect_attr("source"))
                .of(Term::<Entity>::from(entity.clone()))
                .is(Term::<String>::var("__check_source")),
        );
        let pre: Vec<dialog_query::Claim> = branch
            .query()
            .select(source_query.clone())
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(pre.len(), 1, "rule should be installed before retract");

        // Resolve a retract handle from the branch and apply it.
        let resolver = Rule::retracting(entity.clone());
        let resolved = resolver
            .resolve(&Source::from(&branch), &operator)
            .await?
            .expect("installed rule resolves");
        branch
            .transaction()
            .retract(resolved)
            .commit()
            .perform(&operator)
            .await?;

        // The source claim must be gone — proving the dissociate
        // matched stored bytes byte-for-byte. If
        // `Effect::source()` had been used directly (the old
        // `retract_effect` path) any divergence in serialisation
        // would leave this claim in place.
        let post: Vec<dialog_query::Claim> = branch
            .query()
            .select(source_query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            post.is_empty(),
            "dialog.effect/source claim should be gone after retract, saw {post:?}"
        );

        Ok(())
    }

    /// `Rule::asserting` writes the storage-shape claims an
    /// [`AnonymousRuleQuery`] would later read — marker, source,
    /// conclusion, polarity, one `on` entry per attribute. Mirrors
    /// `effect_query::tests::*` but exercises the schema-side
    /// `Statement` impl directly.
    #[dialog_common::test]
    fn it_asserts_the_full_storage_shape() {
        use dialog_artifacts::{Changes, Instruction};

        let inductive =
            InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(inductive);
        let this = effect.this();
        let conclusion = effect.conclusion();
        let on_set = effect.on_entities();

        let mut changes = Changes::default();
        ArtifactsStatement::assert(Rule::asserting(effect.clone()), &mut changes);

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

    /// `Rule::asserting_at` writes the storage-shape claims against
    /// a caller-chosen entity — the marker, source, conclusion,
    /// polarity, and `on` claims must all live at the override
    /// entity rather than the effect's content-derived
    /// [`Effect::this`]. Pins the rule-install-at-named-entity path.
    #[dialog_common::test]
    fn it_asserts_at_a_chosen_entity() {
        use dialog_artifacts::{Changes, Instruction};

        let inductive =
            InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(inductive);
        let derived = effect.this();
        let chosen: Entity = "id:my-counter".parse().expect("id URI parses");
        assert_ne!(
            derived, chosen,
            "test only makes sense when the override differs from the content hash"
        );
        let conclusion = effect.conclusion();
        let on_set = effect.on_entities();

        let mut changes = Changes::default();
        ArtifactsStatement::assert(
            Rule::asserting_at(effect.clone(), chosen.clone()),
            &mut changes,
        );

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
                    && c.of == chosen
                    && matches!(&c.is, Value::Entity(e) if *e == marker)
            }),
            "marker should hang off the chosen entity"
        );
        assert!(
            asserted
                .iter()
                .any(|c| { c.the.to_string() == "dialog.effect/source" && c.of == chosen }),
            "source claim should hang off the chosen entity"
        );
        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/conclusion"
                    && c.of == chosen
                    && matches!(&c.is, Value::Entity(e) if *e == conclusion)
            }),
            "conclusion claim should hang off the chosen entity"
        );
        assert!(
            asserted.iter().any(|c| {
                c.the.to_string() == "dialog.effect/polarity"
                    && c.of == chosen
                    && matches!(&c.is, Value::String(s) if s == "assert")
            }),
            "polarity claim should hang off the chosen entity"
        );
        for attribute in &on_set {
            assert!(
                asserted.iter().any(|c| {
                    c.the.to_string() == "dialog.effect/on"
                        && c.of == chosen
                        && matches!(&c.is, Value::Entity(e) if *e == *attribute)
                }),
                "`on` claim for {attribute} should hang off the chosen entity"
            );
        }
        // No claim lands at the content-derived entity.
        assert!(
            !asserted.iter().any(|c| c.of == derived),
            "no claim should land at the content-derived entity when an override is set"
        );
        // The public accessor returns the override.
        assert_eq!(
            Rule::asserting_at(effect, chosen.clone()).this(),
            chosen,
            "Rule::this() reports the chosen entity"
        );
    }
}
