//! DeductiveRule — the deductive rule-of-rules' `Statement`
//! adapter, parallel to [`Rule`](crate::rule::Rule) for inductive
//! rules.
//!
//! Where [`Rule`](crate::rule::Rule) packages an
//! [`Effect`](tonk_core::effect::Effect) (an inductive rule with
//! polarity) with its `dialog.effect/*` storage shape, this packages
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
//!   serialized as JSON. Source of truth; the rule is rebuilt from
//!   this single text claim.
//! - `db.rule/conclusion` — index pointing at the head concept
//!   entity ([`ConceptDescriptor::this`](dialog_query::ConceptDescriptor::this)).
//!   The [`RuleSource`](dialog_repository::RuleSource) resolver looks
//!   rules up by this: "every rule whose conclusion is concept X".
//! - `dialog.meta/rule` — marker (value `db:rule`), so "all deductive
//!   rules on this branch" has a known triple to start from.
//! - `dialog.meta/description` — optional human-readable description.
//!
//! # Identity
//!
//! The entity is content-addressed:
//! `rule:<base58(blake3(dag-cbor(DeductiveRuleDescriptor)))>`,
//! mirroring [`Effect::this`](tonk_core::effect::Effect::this) (minus
//! polarity, which deductive rules lack). Asserting the same rule
//! twice is idempotent.

use base58::ToBase58;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_common::Blake3Hash;
use dialog_query::{DeductiveRule as CompiledRule, DeductiveRuleDescriptor};
use thiserror::Error;
// `content_entity` hashes the canonical JSON `source_string`, not
// dag-cbor — see the note there for why.

/// A deductive rule installed on a branch — pairs a compiled
/// [`DeductiveRule`](dialog_query::DeductiveRule) with the entity its
/// `db.rule/*` claims are written against and the exact source string
/// stored under `db.rule/source`.
///
/// Construct via [`DeductiveRule::asserting`] for content-addressed
/// installs or [`DeductiveRule::asserting_at`] for installs at a
/// caller-chosen entity.
#[derive(Debug, Clone)]
pub struct DeductiveRule {
    /// The compiled rule — conclusion + analyzed premises.
    rule: CompiledRule,
    /// The entity URI the `db.rule/*` claims are written against.
    /// Defaults to the content-derived [`DeductiveRule::this`]; a
    /// caller can override via [`DeductiveRule::asserting_at`].
    this: Entity,
    /// The exact source string carried on `db.rule/source` (JSON
    /// `DeductiveRuleDescriptor`). Synthesised once at construction so
    /// the `Statement` impl writes and (on retract) dissociates the
    /// same bytes.
    source: String,
}

impl DeductiveRule {
    /// Wrap a compiled rule for an `assert` against its
    /// content-derived entity.
    pub fn asserting(rule: CompiledRule) -> Self {
        let this = content_entity(&rule);
        Self::asserting_at(rule, this)
    }

    /// Wrap a compiled rule for an `assert` against a caller-chosen
    /// entity (e.g. a stable `rule!: this: <entity>` URI).
    pub fn asserting_at(rule: CompiledRule, entity: Entity) -> Self {
        let source = source_string(&rule);
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
    /// claim. The [`RuleSource`](dialog_repository::RuleSource)
    /// resolver indexes rules by this.
    pub fn conclusion(&self) -> Entity {
        self.rule.conclusion().this()
    }

    /// The exact `db.rule/source` string (JSON-encoded
    /// [`DeductiveRuleDescriptor`]).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The compiled rule.
    pub fn rule(&self) -> &CompiledRule {
        &self.rule
    }

    /// Rebuild a compiled [`DeductiveRule`](dialog_query::DeductiveRule)
    /// from a stored `db.rule/source` string.
    ///
    /// Deserialization runs dialog's full rule compilation (type
    /// inference + planning), so a malformed or no-longer-valid stored
    /// rule surfaces as [`DeductiveRuleError::Source`] here rather than
    /// failing deeper in the query engine.
    pub fn from_source(source: &str) -> Result<CompiledRule, DeductiveRuleError> {
        let descriptor: DeductiveRuleDescriptor =
            serde_json::from_str(source).map_err(|e| DeductiveRuleError::Source(e.to_string()))?;
        descriptor
            .compile()
            .map_err(|e| DeductiveRuleError::Compile(e.to_string()))
    }
}

impl dialog_artifacts::Statement for DeductiveRule {
    fn assert(self, update: &mut impl dialog_artifacts::Update) {
        let this = self.this.clone();
        let description = self.rule.descriptor().description.clone();
        let conclusion = self.conclusion();

        // Marker — `(?this, dialog.meta/rule, db:rule)`.
        update.associate_unique(
            meta_attr("dialog.meta", "rule"),
            this.clone(),
            Value::Entity(rule_marker_entity()),
        );
        // Source-of-truth claim.
        update.associate_unique(
            meta_attr("db.rule", "source"),
            this.clone(),
            Value::String(self.source),
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
                meta_attr("dialog.meta", "description"),
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
            meta_attr("dialog.meta", "rule"),
            this.clone(),
            Value::Entity(rule_marker_entity()),
        );
        update.dissociate(
            meta_attr("db.rule", "source"),
            this.clone(),
            Value::String(self.source),
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
                meta_attr("dialog.meta", "description"),
                this,
                Value::String(description),
            );
        }
    }
}

/// Errors rehydrating a deductive rule from its stored source.
#[derive(Debug, Error)]
pub enum DeductiveRuleError {
    /// The stored `db.rule/source` string did not parse as a
    /// `DeductiveRuleDescriptor`.
    #[error("deductive rule source parse failed: {0}")]
    Source(String),
    /// The descriptor parsed but failed dialog's rule compilation
    /// (type inference / planning).
    #[error("deductive rule compile failed: {0}")]
    Compile(String),
}

/// Canonical JSON form of the rule descriptor — the value of the
/// `db.rule/source` claim.
///
/// **Canonicalization is load-bearing.** A premise's `where` terms
/// serialize from a `HashMap` ([`Parameters`](dialog_query::Parameters)),
/// whose iteration order is non-deterministic. Serializing the
/// descriptor directly would yield different byte strings for the same
/// rule across compilations, breaking content-addressed identity and
/// idempotent re-seeding. So we round-trip through a
/// [`serde_json::Value`] with every object's keys sorted, giving a
/// stable canonical form. (`serde_json` is built with `preserve_order`
/// here, so sorting overrides insertion order deterministically.)
fn source_string(rule: &CompiledRule) -> String {
    let mut value = serde_json::to_value(rule.descriptor())
        .expect("DeductiveRuleDescriptor always serializes to JSON");
    sort_keys(&mut value);
    serde_json::to_string(&value).expect("a serde_json::Value always re-serializes")
}

/// Recursively sort every object's keys so serialization is a pure
/// function of the value, independent of map iteration order.
fn sort_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let mut child = map.remove(&key).expect("key present");
                sort_keys(&mut child);
                sorted.insert(key, child);
            }
            *map = sorted;
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

/// Content-addressed entity for a compiled rule:
/// `rule:<base58(blake3(canonical-json(descriptor)))>`.
///
/// Hashes the canonical (key-sorted) JSON stored under
/// `db.rule/source`, so identity is a pure function of the stored
/// bytes. This deliberately differs from
/// [`Effect::this`](tonk_core::effect::Effect::this), which hashes
/// dag-cbor: a `DeductiveRuleDescriptor` does not dag-cbor encode
/// (its premise propositions serialize as untagged structs that
/// dag-cbor rejects), and hashing the canonical JSON is both
/// sufficient and consistent with what we store.
fn content_entity(rule: &CompiledRule) -> Entity {
    let hash = Blake3Hash::hash(source_string(rule).as_bytes());
    let encoded = hash.as_bytes().as_ref().to_base58();
    format!("rule:{encoded}")
        .parse()
        .expect("rule:<base58> is a valid entity URI")
}

/// Build a runtime [`Attribute`](dialog_artifacts::Attribute) from a
/// domain + local name.
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

/// Marker entity asserted as the value of `dialog.meta/rule` on every
/// deductive-rule entity. Same role `db:effect` plays for effects and
/// `db:concept` for concepts.
fn rule_marker_entity() -> Entity {
    "db:rule".parse().expect("`db:rule` is a valid entity URI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::{Changes, Statement as _};

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
        // Recompiling the rebuilt rule's descriptor yields the same
        // source string — the body survived the round-trip.
        assert_eq!(source_string(&rebuilt), stored.source());
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
                .any(|(e, a)| *e == this && a == "dialog.meta/rule"),
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
