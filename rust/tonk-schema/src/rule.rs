//! Stored rules — dialog's native `dialog.rule/*` facts.
//!
//! Installing and uninstalling a rule is plain assertion of the
//! compiled dialog rule: `tx.assert(&rule)` stages the discovery /
//! trigger indexes alongside the canonical dag-cbor source, and
//! `tx.retract(&rule)` erases the same facts byte-exactly (the
//! encoding is canonical, so a decoded rule re-encodes to the stored
//! bytes). The rule entity is the content address of its body, which
//! is what makes those facts safe to accept from the ordinary write
//! path — a reader verifies the decoded body against the entity it
//! was stored under.
//!
//! What remains here is the one thing dialog does not provide:
//! resolving a *stored* rule back off a branch by its entity, for the
//! `rule!: this: <entity> ..: _` retract form that addresses a rule
//! without restating its body.

use dialog_artifacts::{Entity, Value};
use dialog_query::{Output as _, Term};
use thiserror::Error;

pub use dialog_query::rule::inductive::Polarity;
pub use dialog_query::rule::statement::{on_entities, reads_entities, source_attr};
pub use dialog_query::{DeductiveRule, InductiveRule, Rule};

use crate::concept::QueryEnv;
use crate::query_source::Source;

/// Resolve the rule stored at `entity` — inductive or deductive,
/// whichever the stored body decodes as.
pub fn stored_rule(entity: Entity) -> StoredRule {
    StoredRule { entity }
}

/// Builder for [`stored_rule`]. Reads the `dialog.rule/source` claim
/// off a branch and rehydrates the compiled rule.
#[derive(Debug, Clone)]
pub struct StoredRule {
    entity: Entity,
}

impl StoredRule {
    /// Resolve against a branch. Returns `None` when the entity has no
    /// `dialog.rule/source` claim (no such rule installed), so a
    /// retract of something absent drops silently rather than erroring.
    pub async fn resolve<Env: QueryEnv>(
        self,
        source: &Source<'_>,
        env: &Env,
    ) -> Result<Option<Rule>, StoredRuleError> {
        let the: dialog_query::attribute::The = "dialog.rule/source"
            .parse()
            .expect("`dialog.rule/source` is a valid attribute URI");
        let source_claims: Vec<dialog_query::Claim> = source
            .select(dialog_query::AttributeQuery::from(
                Term::<dialog_query::attribute::The>::from(the)
                    .of(Term::<Entity>::from(self.entity.clone()))
                    .is(Term::<Vec<u8>>::var("__source")),
            ))
            .perform(env)
            .try_vec()
            .await
            .map_err(|e| StoredRuleError::Query(format!("source lookup failed: {e:?}")))?;
        let Some(source_claim) = source_claims.into_iter().next() else {
            return Ok(None);
        };
        let Value::Bytes(source_bytes) = source_claim.is else {
            return Err(StoredRuleError::Storage(
                "dialog.rule/source claim was not bytes".to_owned(),
            ));
        };

        // The two kinds share the source attribute; the decoded
        // descriptor's head field decides which one this is.
        if let Ok(rule) = InductiveRule::decode(&source_bytes) {
            return Ok(Some(Rule::Inductive(rule)));
        }
        match DeductiveRule::decode(&source_bytes) {
            Ok(rule) => Ok(Some(Rule::Deductive(rule))),
            Err(reason) => Err(StoredRuleError::Storage(format!(
                "stored rule body did not decode as either kind: {reason}"
            ))),
        }
    }
}

/// Errors resolving a stored rule: either the branch query plumbing
/// failed, or the stored facts are malformed.
#[derive(Debug, Error)]
pub enum StoredRuleError {
    /// The branch query infrastructure returned an error.
    #[error("rule resolve query failed: {0}")]
    Query(String),
    /// A stored claim had the wrong shape — wrong value kind or a
    /// body that does not decode.
    #[error("rule storage shape: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_artifacts::{Changes, Instruction};
    use dialog_query::{Cardinality, ConceptDescriptor};

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    fn descriptor(name: &str) -> ConceptDescriptor {
        serde_json::from_value(serde_json::json!({
            "with": {
                "tag": { "the": format!("{name}/tag"), "as": "Text", "cardinality": "one" }
            }
        }))
        .expect("descriptor parses")
    }

    fn premise(name: &str, var: &str) -> dialog_query::Premise {
        let mut terms = dialog_query::Parameters::default();
        terms.insert("this".into(), Term::<Entity>::var("this").into());
        terms.insert("tag".into(), Term::<String>::var(var).into());
        dialog_query::Premise::Assert(dialog_query::Proposition::Concept(
            dialog_query::ConceptQuery {
                terms,
                predicate: descriptor(name),
            },
        ))
    }

    /// An asserted rule stages the native `dialog.rule/*` facts:
    /// the shared source body, plus the kind's own index claims.
    #[dialog_common::test]
    fn it_asserts_the_native_storage_shape() {
        let rule = InductiveRule::new(
            descriptor("io.gozala.pong"),
            vec![premise("io.gozala.ping", "tag")],
        )
        .expect("rule compiles");
        let this = rule.this();

        let mut changes = Changes::new();
        dialog_artifacts::Statement::assert(&rule, &mut changes);
        let instructions = changes.into_instructions();

        let source: dialog_artifacts::Attribute = "dialog.rule/source".parse().unwrap();
        let induces: dialog_artifacts::Attribute = "dialog.rule/induces".parse().unwrap();
        assert!(instructions.iter().any(|i| matches!(
            i,
            Instruction::Assert(a) if a.the == source && a.of == this
        )));
        assert!(instructions.iter().any(|i| matches!(
            i,
            Instruction::Assert(a) if a.the == induces && a.of == this
        )));

        let _ = Cardinality::One; // keep the shared import used across cfgs
    }

    /// Decoding dispatches on the stored body's head field: an
    /// inductive body comes back inductive, a deductive one deductive.
    #[dialog_common::test]
    fn it_round_trips_both_kinds_through_the_stored_source() {
        let inductive = InductiveRule::new(
            descriptor("io.gozala.pong"),
            vec![premise("io.gozala.ping", "tag")],
        )
        .expect("inductive compiles");
        let decoded = InductiveRule::decode(&inductive.encode()).expect("decodes");
        assert_eq!(decoded.this(), inductive.this());

        let deductive = DeductiveRule::new(
            descriptor("io.gozala.pong"),
            vec![premise("io.gozala.ping", "tag")],
        )
        .expect("deductive compiles");
        assert!(
            InductiveRule::decode(&deductive.encode()).is_err(),
            "a deductive body must not decode as inductive"
        );
        let decoded = DeductiveRule::decode(&deductive.encode()).expect("decodes");
        assert_eq!(decoded.this(), deductive.this());
    }
}
