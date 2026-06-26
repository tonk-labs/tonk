//! Deductive-rule resolution for branch queries.
//!
//! Deductive rules are stored as facts on a branch (the `db.rule/*`
//! claim shape, see [`tonk_schema::deductive_rule`]). When a concept
//! is queried, dialog asks its installed
//! [`RuleSource`](dialog_repository::RuleSource) for the rules
//! concluding that concept. This module is the reactor's
//! implementation: it reads the rule facts through the query's own
//! branch+overlay union, hydrates them, and caches the result per
//! concept so repeat queries pay no scan.
//!
//! # Cache
//!
//! [`ConceptCache`] lives on a [`BranchState`](crate::BranchState).
//! Each entry records the [`TreeReference`] (branch head) it was
//! scanned at alongside the hydrated rules; a lookup that finds the
//! recorded head still current returns the cached rules with no tree
//! scan, while a head advance on *that concept's* entry triggers a
//! single re-scan. Rules are keyed within an entry by their
//! content-addressed entity, so a body shared across concepts or
//! surviving an unrelated head change is reused without re-hydrating.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Value};
use dialog_query::DeductiveRule as CompiledRule;
use dialog_query::concept::descriptor::ConceptDescriptor;
use dialog_query::concept::query::ConceptRules;
use dialog_query::error::EvaluationError;
use dialog_repository::{RuleClaims, RuleSource, TreeReference};
use parking_lot::RwLock;

use tonk_schema::deductive_rule::DeductiveRule as StoredRule;

/// Hydrated deductive rules for one concept, tagged with the branch
/// head they were scanned at.
#[derive(Default)]
struct RuleCache {
    /// Branch head this entry was scanned at. A lookup compares it
    /// against the current head to decide fresh-vs-stale.
    tree: TreeReference,
    /// Hydrated rules keyed by their content-addressed entity
    /// (`rule:<hash>`), so unchanged rules are reused across re-scans.
    rules: HashMap<Entity, CompiledRule>,
}

/// Per-branch cache of resolved deductive rules, keyed by conclusion
/// concept entity. Held on [`BranchState`](crate::BranchState).
#[derive(Default)]
pub struct ConceptCache {
    concepts: RwLock<HashMap<Entity, RuleCache>>,
}

impl ConceptCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached rules for `concept` if the entry was scanned
    /// at `head`; otherwise `None` (caller must re-scan).
    fn get_fresh(&self, concept: &Entity, head: &TreeReference) -> Option<Vec<CompiledRule>> {
        let concepts = self.concepts.read();
        let entry = concepts.get(concept)?;
        if &entry.tree == head {
            Some(entry.rules.values().cloned().collect())
        } else {
            None
        }
    }

    /// Replace the cached entry for `concept` with `rules` scanned at
    /// `head`. Reuses already-hydrated rule bodies by entity so a
    /// re-scan doesn't re-pay hydration for unchanged rules.
    fn store(&self, concept: Entity, head: TreeReference, rules: HashMap<Entity, CompiledRule>) {
        self.concepts
            .write()
            .insert(concept, RuleCache { tree: head, rules });
    }

    /// Snapshot of an existing entry's hydrated bodies, so a re-scan
    /// can reuse rules whose entity (content hash) is unchanged.
    fn hydrated(&self, concept: &Entity) -> HashMap<Entity, CompiledRule> {
        self.concepts
            .read()
            .get(concept)
            .map(|entry| entry.rules.clone())
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
    /// Current branch head, for the cache freshness check.
    head: TreeReference,
}

impl ReactorRuleSource {
    /// Build a rule source over `cache`, treating `head` as the
    /// current branch revision for freshness.
    pub fn new(cache: Arc<ConceptCache>, head: TreeReference) -> Self {
        Self { cache, head }
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

        // Fast path: cache holds rules scanned at the current head.
        if let Some(cached) = self.cache.get_fresh(&concept_entity, &self.head) {
            for rule in cached {
                rules.install(rule);
            }
            return Ok(rules);
        }

        // Slow path: find rule entities whose conclusion is this
        // concept, via the same branch+overlay union the query reads.
        let conclusion_claims = claims
            .select_claims(
                ArtifactSelector::new()
                    .the(rule_attr("conclusion"))
                    .is(Value::Entity(concept_entity.clone())),
            )
            .await
            .map_err(|e| EvaluationError::Store(format!("rule conclusion lookup: {e:?}")))?;

        // Nothing concludes this concept: cache the empty result so we
        // don't re-scan until the head advances, and return implicit-only.
        if conclusion_claims.is_empty() {
            self.cache
                .store(concept_entity, self.head.clone(), HashMap::new());
            return Ok(rules);
        }

        // Reuse bodies already hydrated for this concept (unchanged
        // rules across a head advance); hydrate the rest from source.
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
        self.cache
            .store(concept_entity, self.head.clone(), resolved);

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
        let head = branch
            .revision()
            .map(|revision| revision.tree)
            .unwrap_or_default();

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .with_rules(Arc::new(ReactorRuleSource::new(
                cache.clone(),
                head.clone(),
            )))
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

        // Second query reuses the cache (same head) — still resolves.
        let again: Vec<ConceptConclusion> = branch
            .query()
            .with_rules(Arc::new(ReactorRuleSource::new(cache, head)))
            .select(query)
            .perform(&operator)
            .try_vec()
            .await?;
        assert!(again.iter().any(|c| *c.entity() == alice));

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
