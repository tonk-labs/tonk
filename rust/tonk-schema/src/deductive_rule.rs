//! DeductiveRule — the deductive rule-of-rules' `Statement`
//! adapter, parallel to [`Rule`](crate::rule::Rule) for inductive
//! rules.
//!
//! Where [`Rule`](crate::rule::Rule) packages an
//! [`Effect`](tonk_core::effect::Effect) (an inductive rule with
//! polarity) with its `db.effect/*` storage shape, this packages
//! a [`DeductiveRule`](dialog_query::DeductiveRule) with a
//! `db.rule/*` storage shape so a deductive `rule!:` (the `assert:`
//! no-bang form) flows through dialog's `Statement` trait.
//!
//! # Inductive vs deductive
//!
//! An inductive rule *fires on commit*, asserting head facts into the
//! transaction. A deductive rule *derives on query*: it has no
//! polarity and writes nothing — it is consulted when its conclusion
//! concept is queried (via the
//! [`RuleSource`](dialog_repository::RuleSource) seam). So the
//! storage shape drops `polarity` and the `on` reverse index (which
//! the reactor's fixpoint needs but a query-time resolver does not),
//! keeping only the source-of-truth body and a conclusion index.
//!
//! # Storage shape
//!
//! Each deductive-rule entity carries:
//!
//! - `db.rule/source` — the full
//!   [`DeductiveRuleDescriptor`](dialog_query::DeductiveRuleDescriptor)
//!   as canonical dag-cbor (a `Value::Bytes`). Source of truth; the
//!   rule is rebuilt from this single claim via
//!   [`DeductiveRule::decode`](dialog_query::DeductiveRule::decode).
//! - `db.rule/conclusion` — index pointing at the head concept
//!   entity ([`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this)).
//!   The [`RuleSource`](dialog_repository::RuleSource) resolver looks
//!   rules up by this: "every rule whose conclusion is concept X".
//! - `db.meta/rule` — marker (value `db:rule`), so "all deductive
//!   rules on this branch" has a known triple to start from.
//! - `db.meta/description` — optional human-readable description.
//!
//! # Identity
//!
//! The entity is content-addressed:
//! `rule:<base58(blake3(dag-cbor(DeductiveRuleDescriptor)))>`,
//! mirroring [`Effect::this`](tonk_core::effect::Effect::this) (minus
//! polarity, which deductive rules lack). Asserting the same rule
//! twice is idempotent.

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_query::DeductiveRule as CompiledRule;
use dialog_query::{Output as _, Term, the};
use thiserror::Error;

use crate::concept::QueryEnv;
use crate::query_source::Source;

/// A deductive rule installed on a branch — pairs a compiled
/// [`DeductiveRule`](dialog_query::DeductiveRule) with the entity its
/// `db.rule/*` claims are written against and the canonical dag-cbor
/// body stored under `db.rule/source`.
///
/// Identity and encoding are owned by dialog-query
/// ([`DeductiveRule::this`](dialog_query::DeductiveRule::this) /
/// [`encode`](dialog_query::DeductiveRule::encode)); this type just
/// projects them onto the `db.rule/*` claim shape.
///
/// Construct via [`DeductiveRule::asserting`] for content-addressed
/// installs or [`DeductiveRule::asserting_at`] for installs at a
/// caller-chosen entity.
#[derive(Debug, Clone)]
pub struct DeductiveRule {
    /// The compiled rule — conclusion + analyzed premises.
    rule: CompiledRule,
    /// The entity URI the `db.rule/*` claims are written against.
    /// Defaults to the content-derived
    /// [`DeductiveRule::this`](dialog_query::DeductiveRule::this); a
    /// caller can override via [`DeductiveRule::asserting_at`].
    this: Entity,
    /// The canonical dag-cbor body carried on `db.rule/source`.
    /// Captured once at construction so `assert` writes and `retract`
    /// dissociates the same bytes.
    source: Vec<u8>,
}

impl DeductiveRule {
    /// Wrap a compiled rule for an `assert` against its
    /// content-derived entity ([`DeductiveRule::this`](dialog_query::DeductiveRule::this)).
    pub fn asserting(rule: CompiledRule) -> Self {
        let this = rule.this();
        Self::asserting_at(rule, this)
    }

    /// Wrap a compiled rule for an `assert` against a caller-chosen
    /// entity (e.g. a stable `rule!: this: <entity>` URI).
    pub fn asserting_at(rule: CompiledRule, entity: Entity) -> Self {
        let source = rule.encode();
        Self {
            rule,
            this: entity,
            source,
        }
    }

    /// The entity URI the `db.rule/*` claims live under.
    pub fn this(&self) -> Entity {
        self.this.clone()
    }

    /// The head concept entity — the value of the `db.rule/conclusion`
    /// claim. The resolver indexes rules by this.
    pub fn conclusion(&self) -> Entity {
        self.rule.conclusion().this()
    }

    /// The canonical dag-cbor `db.rule/source` bytes.
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// The compiled rule.
    pub fn rule(&self) -> &CompiledRule {
        &self.rule
    }

    /// Rebuild a compiled [`DeductiveRule`](dialog_query::DeductiveRule)
    /// from stored `db.rule/source` dag-cbor bytes.
    ///
    /// Decoding runs dialog's full rule compilation (type inference +
    /// planning), so a malformed or no-longer-valid stored rule surfaces
    /// as [`DeductiveRuleError`] here rather than deeper in the engine.
    pub fn from_source(source: &[u8]) -> Result<CompiledRule, DeductiveRuleError> {
        CompiledRule::decode(source).map_err(DeductiveRuleError)
    }

    /// Begin building a [`DeductiveRule`] for a `retract` mutation. The
    /// returned [`Retracting`] resolves against a branch to read the
    /// stored `db.rule/source` dag-cbor and bake those exact bytes into
    /// the resulting rule, so the dissociate removes the same EAVs that
    /// were written. The deductive counterpart to
    /// [`Rule::retracting`](crate::rule::Rule::retracting) — there is no
    /// polarity to recover.
    pub fn retracting(entity: Entity) -> Retracting {
        Retracting { entity }
    }
}

/// A deductive rule resolved off a branch for retraction. Built by
/// [`DeductiveRule::retracting`]; carries only the target entity until
/// [`resolve`](Self::resolve) reads the stored body.
#[derive(Debug, Clone)]
pub struct Retracting {
    entity: Entity,
}

impl Retracting {
    /// Resolve against a branch. Returns `None` when the entity has no
    /// `db.rule/source` claim (no such deductive rule installed), so a
    /// retract of something absent drops silently rather than erroring.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<DeductiveRule>, DeductiveRuleResolveError> {
        let source_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(rule_source_attr())
                    .of(Term::<Entity>::from(self.entity.clone()))
                    .is(Term::<Vec<u8>>::var("__source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| {
                DeductiveRuleResolveError::Query(format!("source lookup failed: {e:?}"))
            })?;
        let Some(source_claim) = source_claims.into_iter().next() else {
            return Ok(None);
        };
        let Value::Bytes(source_bytes) = source_claim.is else {
            return Err(DeductiveRuleResolveError::Storage(
                "db.rule/source claim was not bytes".to_owned(),
            ));
        };

        // Rehydrate the compiled rule from the stored source so
        // `conclusion()` etc. line up with what was written. The carried
        // `source` field stays the *exact* stored bytes so the dissociate
        // targets the same EAVs (including content-addressed installs).
        let rule = DeductiveRule::from_source(&source_bytes).map_err(|e| {
            DeductiveRuleResolveError::Storage(format!("rule rehydrate failed: {e}"))
        })?;

        Ok(Some(DeductiveRule {
            rule,
            // The stored claims live at `self.entity` regardless of what
            // `rule.this()` would hash to from the rehydrated descriptor.
            this: self.entity,
            source: source_bytes,
        }))
    }
}

/// Errors resolving an installed deductive rule for retraction: either
/// the branch query plumbing failed, or the stored facts are malformed.
/// The deductive counterpart to
/// [`RuleResolveError`](crate::rule::RuleResolveError).
#[derive(Debug, Error)]
pub enum DeductiveRuleResolveError {
    /// The branch query infrastructure returned an error.
    #[error("deductive rule resolve query failed: {0}")]
    Query(String),
    /// A stored claim had the wrong shape — wrong value kind, or a
    /// `db.rule/source` body that no longer decodes.
    #[error("deductive rule storage shape: {0}")]
    Storage(String),
}

/// The `db.rule/source` attribute as a query [`The`](dialog_query::attribute::The).
fn rule_source_attr() -> dialog_query::attribute::The {
    the!("db.rule/source")
}

impl dialog_artifacts::Statement for DeductiveRule {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        let this = self.this.clone();
        let description = self.rule.descriptor().description.clone();
        let conclusion = self.conclusion();

        // Marker — `(?this, db.meta/rule, db:rule)`.
        update.associate_unique(
            meta_attr("db.meta", "rule"),
            this.clone(),
            Value::Entity(rule_marker_entity()),
        );
        // Source-of-truth claim.
        update.associate_unique(
            meta_attr("db.rule", "source"),
            this.clone(),
            Value::Bytes(self.source),
        );
        // Conclusion index.
        update.associate_unique(
            meta_attr("db.rule", "conclusion"),
            this.clone(),
            Value::Entity(conclusion),
        );
        // Optional description.
        if let Some(description) = description
            && !description.is_empty()
        {
            update.associate_unique(
                meta_attr("db.meta", "description"),
                this,
                Value::String(description),
            );
        }
    }

    fn retract(self, update: &mut impl dialog_artifacts::Update) {
        let this = self.this.clone();
        let description = self.rule.descriptor().description.clone();
        let conclusion = self.conclusion();

        update.dissociate(
            meta_attr("db.meta", "rule"),
            this.clone(),
            Value::Entity(rule_marker_entity()),
        );
        update.dissociate(
            meta_attr("db.rule", "source"),
            this.clone(),
            Value::Bytes(self.source),
        );
        update.dissociate(
            meta_attr("db.rule", "conclusion"),
            this.clone(),
            Value::Entity(conclusion),
        );
        if let Some(description) = description
            && !description.is_empty()
        {
            update.dissociate(
                meta_attr("db.meta", "description"),
                this,
                Value::String(description),
            );
        }
    }
}

/// Error rehydrating a deductive rule from its stored `db.rule/source`
/// bytes — the dag-cbor failed to decode, or the decoded descriptor
/// didn't compile (type inference / planning). Carries dialog's reason.
#[derive(Debug, Error)]
#[error("deductive rule hydrate failed: {0}")]
pub struct DeductiveRuleError(String);

impl From<String> for DeductiveRuleError {
    fn from(reason: String) -> Self {
        Self(reason)
    }
}

/// Build a runtime [`Attribute`](dialog_artifacts::Attribute) from a
/// domain + local name.
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

/// Marker entity asserted as the value of `db.meta/rule` on every
/// deductive-rule entity. Same role `db:effect` plays for effects and
/// `db:concept` for concepts.
fn rule_marker_entity() -> Entity {
    "db:rule".parse().expect("`db:rule` is a valid entity URI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::{Changes, Statement as _};
    use dialog_query::DeductiveRuleDescriptor;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A real deductive rule with a concept-premise body, built the
    /// way the analyzer will build them (descriptor -> compile). The
    /// implicit rule (`DeductiveRule::from(&concept)`) is *not* usable
    /// as a fixture here: its body is raw `AttributeQuery` premises,
    /// which the descriptor's formal-notation `Serialize` rejects.
    /// Stored deductive rules always have concept/formula bodies.
    fn ingredient_rule() -> CompiledRule {
        let json = serde_json::json!({
            "deduce": {
                "with": {
                    "name": { "the": "diy.cook/ingredient-name", "as": "Text" }
                }
            },
            "when": [
                {
                    "assert": {
                        "with": {
                            "name": { "the": "diy.cook/ingredient-name", "as": "Text" }
                        }
                    },
                    "where": {
                        "this": { "?": { "name": "this" } },
                        "name": { "?": { "name": "name" } }
                    }
                }
            ]
        });
        let descriptor: DeductiveRuleDescriptor =
            serde_json::from_value(json).expect("fixture descriptor parses");
        descriptor.compile().expect("fixture rule compiles")
    }

    #[dialog_common::test]
    fn it_round_trips_through_source() {
        let rule = ingredient_rule();
        let stored = DeductiveRule::asserting(rule.clone());
        let rebuilt =
            DeductiveRule::from_source(stored.source()).expect("stored source rehydrates");
        // Re-encoding the rebuilt rule yields the same dag-cbor bytes —
        // the body survived the round-trip.
        assert_eq!(rebuilt.encode(), stored.source());
        assert_eq!(rebuilt.this(), rule.this());
    }

    #[dialog_common::test]
    fn it_is_content_addressed_and_idempotent() {
        let a = DeductiveRule::asserting(ingredient_rule());
        let b = DeductiveRule::asserting(ingredient_rule());
        assert_eq!(a.this(), b.this(), "same rule hashes to the same entity");
        assert!(
            a.this().to_string().starts_with("rule:"),
            "deductive rule entity is a rule: URI"
        );
    }

    #[dialog_common::test]
    fn it_indexes_by_conclusion_concept() {
        let rule = ingredient_rule();
        let conclusion = rule.conclusion().this();
        let stored = DeductiveRule::asserting(rule);
        assert_eq!(
            stored.conclusion(),
            conclusion,
            "conclusion index points at the head concept entity"
        );
    }

    #[dialog_common::test]
    fn it_emits_the_expected_claim_shape() {
        let stored = DeductiveRule::asserting(ingredient_rule());
        let this = stored.this();
        let conclusion = stored.conclusion();

        let mut changes = Changes::new();
        stored.assert(&mut changes);

        let claims: Vec<_> = changes
            .iter()
            .map(|(entity, attr, _)| (entity.clone(), attr.to_string()))
            .collect();

        // Marker, source, and conclusion all written against `this`.
        assert!(
            claims
                .iter()
                .any(|(e, a)| *e == this && a == "db.meta/rule"),
            "marker claim present"
        );
        assert!(
            claims
                .iter()
                .any(|(e, a)| *e == this && a == "db.rule/source"),
            "source claim present"
        );
        assert!(
            claims
                .iter()
                .any(|(e, a)| *e == this && a == "db.rule/conclusion"),
            "conclusion index present"
        );
        // Sanity: the conclusion is the counter concept, not `this`.
        assert_ne!(conclusion, this);
    }
}
