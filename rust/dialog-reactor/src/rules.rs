//! Deductive-rule resolution for branch queries.
//!
//! Deductive rules are stored as facts on a branch (the `db.rule/*`
//! claim shape, see [`tonk_schema::deductive_rule`]). When a concept
//! is queried, dialog asks its installed
//! [`RuleSource`](dialog_repository::RuleSource) for the rules
//! concluding that concept. This module is the reactor's
//! implementation: it reads the rule facts through the query's own
//! branch+overlay union, hydrates them, and reuses already-built rule
//! bodies via a per-branch hydration cache.
//!
//! # Cache: hydration only, never a "skip the scan" cache
//!
//! [`ConceptCache`] lives on a [`BranchState`](crate::BranchState). It
//! maps each conclusion concept to the compiled rule bodies already
//! built, keyed by content-addressed rule entity. It does NOT decide
//! *which* rules apply: the conclusion lookup is re-run through the
//! branch+overlay union on every resolve, so a rule asserted into the
//! overlay (`tx.assert(rule)` / `.with(rule)`, uncommitted — the branch
//! head has not moved) is always seen. A head-keyed "skip the scan"
//! cache would hit on the unchanged head and silently ignore such a
//! rule. The cache only avoids re-paying CBOR decode + recompile for a
//! body we already built; content addressing means a cached body is
//! never stale.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Value};
use dialog_query::DeductiveRule as CompiledRule;
use dialog_query::concept::descriptor::ConceptDescriptor;
use dialog_query::concept::query::ConceptRules;
use dialog_query::error::EvaluationError;
use dialog_repository::{RuleClaims, RuleSource};
use parking_lot::RwLock;

use tonk_schema::deductive_rule::DeductiveRule as StoredRule;

/// Per-branch HYDRATION cache: for each conclusion concept, the
/// compiled rule bodies already built, keyed by their
/// content-addressed entity (`rule:<hash>`). Held on
/// [`BranchState`](crate::BranchState).
///
/// This is deliberately NOT a "which rules apply" cache keyed by
/// branch head: the conclusion lookup is always re-run through the
/// branch+overlay union (so an overlay-asserted, uncommitted rule is
/// seen — the head hasn't moved). The cache only avoids re-paying CBOR
/// decode + recompile for a rule body we've already built. Content
/// addressing means a cached body is never stale.
#[derive(Default)]
pub struct ConceptCache {
    concepts: RwLock<HashMap<Entity, HashMap<Entity, CompiledRule>>>,
}

impl ConceptCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the cached hydrated bodies for `concept`.
    fn store(&self, concept: Entity, rules: HashMap<Entity, CompiledRule>) {
        self.concepts.write().insert(concept, rules);
    }

    /// Snapshot of an existing entry's hydrated bodies, so a re-scan
    /// reuses rules whose entity (content hash) is unchanged.
    fn hydrated(&self, concept: &Entity) -> HashMap<Entity, CompiledRule> {
        self.concepts
            .read()
            .get(concept)
            .cloned()
            .unwrap_or_default()
    }
}

/// Build the `db.rule/<name>` attribute used in rule-fact selectors.
fn rule_attr(name: &str) -> Attribute {
    format!("db.rule/{name}")
        .parse()
        .expect("db.rule/<name> is a valid attribute URI")
}

/// A [`RuleSource`](dialog_repository::RuleSource) backed by a
/// branch's stored deductive rules and its [`ConceptCache`].
///
/// Constructed per query (cheap — it just clones the `Arc`s) and
/// handed to `QueryLayer::with_rules`. The cache it reads/writes is
/// shared with the branch, so resolution work done by one query
/// benefits later ones.
pub struct ReactorRuleSource {
    cache: Arc<ConceptCache>,
}

impl ReactorRuleSource {
    /// Build a rule source over the branch's shared hydration cache.
    pub fn new(cache: Arc<ConceptCache>) -> Self {
        Self { cache }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl RuleSource for ReactorRuleSource {
    async fn resolve(
        &self,
        concept: &ConceptDescriptor,
        mut rules: ConceptRules,
        claims: &dyn RuleClaims,
    ) -> Result<ConceptRules, EvaluationError> {
        let concept_entity = concept.this();

        // Always run the conclusion lookup through the branch+overlay
        // union the query reads — this is what makes a rule asserted into
        // the overlay (`tx.assert(rule)` / `.with(rule)`, uncommitted, so
        // the branch head has NOT moved) visible. The cache must NOT be
        // used to skip this scan: a head-keyed cache would hit on the
        // unchanged head and silently ignore an overlay rule (or, having
        // cached an empty result, mask it). So the cache below is a pure
        // *hydration* cache (skip CBOR decode + recompile for a
        // content-addressed rule we already built), never a "skip the
        // scan" cache. The expensive part (hydrate) is still cached; the
        // cheap part (the indexed union select) always runs.
        let conclusion_claims = claims
            .select_claims(
                ArtifactSelector::new()
                    .the(rule_attr("conclusion"))
                    .is(Value::Entity(concept_entity.clone())),
            )
            .await
            .map_err(|e| EvaluationError::Store(format!("rule conclusion lookup: {e:?}")))?;

        if conclusion_claims.is_empty() {
            return Ok(rules);
        }

        // Reuse already-hydrated bodies by rule entity (content hash), so
        // a rule seen before isn't re-decoded/re-compiled; hydrate the
        // rest from source. Entries are keyed by rule.this() which is a
        // content hash, so a cached body is never stale.
        let mut prior = self.cache.hydrated(&concept_entity);
        let mut resolved: HashMap<Entity, CompiledRule> = HashMap::new();

        for claim in conclusion_claims {
            let rule_entity = claim.of;
            let compiled = if let Some(existing) = prior.remove(&rule_entity) {
                existing
            } else {
                hydrate(&rule_entity, claims).await?
            };
            resolved.insert(rule_entity, compiled);
        }

        for rule in resolved.values() {
            rules.install(rule.clone());
        }
        // Cache the hydrated bodies for reuse (keyed by content-addressed
        // rule entity). This is a hydration cache only — it never gates
        // whether to scan, so overlay rules are always picked up above.
        self.cache.store(concept_entity, resolved);

        Ok(rules)
    }
}

/// Fetch the `db.rule/source` claim for `rule_entity` and rehydrate a
/// compiled [`DeductiveRule`](dialog_query::DeductiveRule).
async fn hydrate(
    rule_entity: &Entity,
    claims: &dyn RuleClaims,
) -> Result<CompiledRule, EvaluationError> {
    let source_claims = claims
        .select_claims(
            ArtifactSelector::new()
                .the(rule_attr("source"))
                .of(rule_entity.clone()),
        )
        .await
        .map_err(|e| EvaluationError::Store(format!("rule source lookup: {e:?}")))?;

    let source = source_claims
        .into_iter()
        .find_map(|claim| match claim.is {
            Value::String(source) => Some(source),
            _ => None,
        })
        .ok_or_else(|| {
            EvaluationError::Store(format!("rule {rule_entity} missing db.rule/source claim"))
        })?;

    StoredRule::from_source(&source)
        .map_err(|e| EvaluationError::Store(format!("rule {rule_entity} hydrate: {e}")))
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    use super::*;
    use dialog_query::concept::descriptor::{ConceptConclusion, ConceptDescriptor};
    use dialog_query::concept::query::ConceptQuery;
    use dialog_query::{Output as _, Parameters, Term, the};
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};

    /// The `employee` conclusion concept: derived, one `name` field.
    fn employee_descriptor() -> ConceptDescriptor {
        serde_json::from_value(serde_json::json!({
            "with": { "name": { "the": "org/employee-name", "as": "Text" } }
        }))
        .expect("employee descriptor parses")
    }

    /// A deductive rule: an `employee` is anyone with an
    /// `org/person-name` fact, projected as `employee-name`.
    fn employee_from_person() -> StoredRule {
        let json = serde_json::json!({
            "deduce": {
                "with": { "name": { "the": "org/employee-name", "as": "Text" } }
            },
            "when": [
                {
                    "assert": {
                        "with": { "name": { "the": "org/person-name", "as": "Text" } }
                    },
                    "where": {
                        "this": { "?": { "name": "this" } },
                        "name": { "?": { "name": "name" } }
                    }
                }
            ]
        });
        let descriptor: dialog_query::DeductiveRuleDescriptor =
            serde_json::from_value(json).expect("rule descriptor parses");
        StoredRule::asserting(descriptor.compile().expect("rule compiles"))
    }

    /// End-to-end: a deductive rule stored as `db.rule/*` facts is
    /// resolved through `ReactorRuleSource` so a query for the
    /// conclusion concept (`employee`) returns rows derived from the
    /// flat data (`org/person-name` facts) — even though no
    /// `employee` fact was ever written.
    #[dialog_common::test]
    async fn it_resolves_a_stored_deductive_rule_on_query() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit a flat person fact plus the deductive rule.
        let alice: dialog_artifacts::Entity = "id:alice".parse()?;
        branch
            .transaction()
            .assert(
                the!("org/person-name")
                    .of(alice.clone())
                    .is("Alice".to_string()),
            )
            .assert(employee_from_person())
            .commit()
            .perform(&operator)
            .await?;

        // Query `employee` — no employee fact exists; the rows can
        // only come from the deductive rule resolved by the source.
        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::var("this"));
        terms.insert("name".into(), Term::var("name"));
        let query = ConceptQuery {
            predicate: employee_descriptor(),
            terms,
        };

        let cache = Arc::new(ConceptCache::new());

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .with_rules(Arc::new(ReactorRuleSource::new(cache.clone())))
            .select(query.clone())
            .perform(&operator)
            .try_vec()
            .await?;

        // Alice surfaces as an employee via the rule.
        assert!(
            conclusions.iter().any(|c| {
                *c.entity() == alice
                    && c.get::<String>("name")
                        .map(|n| n == "Alice")
                        .unwrap_or(false)
            }),
            "expected Alice as a derived employee, got {conclusions:?}"
        );

        // Second query reuses the hydration cache — still resolves.
        let again: Vec<ConceptConclusion> = branch
            .query()
            .with_rules(Arc::new(ReactorRuleSource::new(cache)))
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(again.iter().any(|c| *c.entity() == alice));

        Ok(())
    }

    /// Regression: a rule asserted into the OVERLAY (`.with(rule)`,
    /// uncommitted, so the branch head has NOT moved) must resolve —
    /// even after a prior query of the same concept ran and populated
    /// the cache. A head-keyed "skip the scan" cache would hit on the
    /// unchanged head and silently ignore the overlay rule (the bug
    /// that made the inspector's evaluate preview show no deductions).
    #[dialog_common::test]
    async fn it_resolves_an_overlay_rule_after_a_prior_query() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Commit ONLY the flat person fact — no rule on the branch.
        let alice: dialog_artifacts::Entity = "id:alice".parse()?;
        branch
            .transaction()
            .assert(
                the!("org/person-name")
                    .of(alice.clone())
                    .is("Alice".to_string()),
            )
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::var("this"));
        terms.insert("name".into(), Term::var("name"));
        let query = ConceptQuery {
            predicate: employee_descriptor(),
            terms,
        };

        // The branch's shared cache, reused across both queries below.
        let cache = Arc::new(ConceptCache::new());

        // First query: no rule exists yet. Under the old design this
        // would cache an empty result for `employee` at the current head.
        let before: Vec<ConceptConclusion> = branch
            .query()
            .with_rules(Arc::new(ReactorRuleSource::new(cache.clone())))
            .select(query.clone())
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(before.is_empty(), "no rule yet, got {before:?}");

        // Now add the rule to the OVERLAY (not committed — head is the
        // same as the first query) and re-resolve. It MUST surface.
        let after: Vec<ConceptConclusion> = branch
            .query()
            .with(employee_from_person())
            .with_rules(Arc::new(ReactorRuleSource::new(cache)))
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            after.iter().any(|c| *c.entity() == alice),
            "overlay rule must resolve despite the prior cached query, got {after:?}"
        );

        Ok(())
    }

    /// Without a rule source, the same query returns nothing — the
    /// rule only resolves through `with_rules`.
    #[dialog_common::test]
    async fn it_returns_no_rows_without_the_rule_source() -> anyhow::Result<()> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        let alice: dialog_artifacts::Entity = "id:alice".parse()?;
        branch
            .transaction()
            .assert(
                the!("org/person-name")
                    .of(alice.clone())
                    .is("Alice".to_string()),
            )
            .assert(employee_from_person())
            .commit()
            .perform(&operator)
            .await?;

        let mut terms = Parameters::new();
        terms.insert("this".into(), Term::var("this"));
        terms.insert("name".into(), Term::var("name"));
        let query = ConceptQuery {
            predicate: employee_descriptor(),
            terms,
        };

        // No `.with_rules(..)`: only the implicit rule runs, and there
        // is no stored `employee` fact, so nothing matches.
        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(
            conclusions.is_empty(),
            "without a rule source the deductive rule must not resolve, got {conclusions:?}"
        );

        Ok(())
    }
}
