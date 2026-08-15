//! The `rule:` query — surfacing installed rules as concept rows.
//!
//! [`effect`](crate::effect) holds the pure [`Effect`] data type
//! and its storage-shape projections. This module holds the
//! query side: the [`AnonymousRuleQuery`] that enumerates every
//! inductive rule installed on a branch as a [`ConceptConclusion`]
//! row, the [`RuleDefinition`] view it materialises, and the
//! [`rule_of_rule_descriptor`] dispatch sentinel.
//!
//! `concept.rs` and `builtin.rs` import these — `rule:` is a
//! queryable concept dispatched through `concept::QueryPlan`. This
//! query/definition machinery belongs with `concept` in
//! `tonk-schema`. The effect storage / lookup / evaluation side
//! lives in `tonk_evaluator::effect_query`.

use dialog_artifacts::{Entity, Value};
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::{
    Application, ConceptDescriptor, EvaluationError, Match, Output as _, Parameters, Scope,
    Selection, Term, the, try_stream,
};
use serde::{Deserialize, Serialize};

use crate::effect::{Effect, EffectPolarity};

/// The well-known marker entity asserted as the value of
/// `db.meta/effect` on every effect entity, mirroring how
/// concept entities carry `(?this, db.meta/concept,
/// db:concept)`. Lets "all effects on this branch" queries start
/// from a selectable triple.
fn effect_marker_entity() -> Entity {
    "db:effect"
        .parse()
        .expect("`db:effect` is a valid entity URI")
}

/// JSON-serialisable view of a rule's definition — the value of an
/// [`AnonymousRuleQuery`] row's synthesised `definition` field.
///
/// Pairs the rule's [`InductiveRuleDescriptor`](dialog_query::InductiveRuleDescriptor)
/// (head conclusion, `when` premises, `unless` premises) with its
/// [`EffectPolarity`] so a `rule:` query surfaces what the rule
/// does, mirroring how `concept:` puts a concept's descriptor in
/// its `source` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDefinition {
    /// The inductive rule descriptor: head concept + premises.
    pub rule: dialog_query::InductiveRuleDescriptor,
    /// Whether the head asserts or retracts when the body matches.
    pub polarity: EffectPolarity,
}

impl RuleDefinition {
    /// Build the definition view from an installed [`Effect`].
    fn from_effect(effect: &Effect) -> Self {
        Self {
            rule: effect.descriptor(),
            polarity: effect.polarity(),
        }
    }
}

/// Custom query application that surfaces every inductive rule
/// installed on a branch as a [`ConceptConclusion`].
///
/// Rules are reified as `db.effect/*` facts (see the
/// [`effect`](crate::effect) module docs). This query enumerates
/// every entity carrying the `db.meta/effect = db:effect`
/// marker, rehydrates each via its `db.effect/*` claims, and
/// materialises its definition into a synthesised `definition`
/// field alongside `this` (the effect entity). It is the
/// rule-side parallel of
/// [`AnonymousConceptQuery`](crate::concept::AnonymousConceptQuery):
/// same [`Application`] shape, same `terms` filter/emit
/// convention.
///
/// `terms` parameter keys map to the user's variable names:
/// `this` (the effect entity) and `definition` (the
/// [`RuleDefinition`] as a JSON string). Constant terms filter;
/// variable terms bind.
#[derive(Debug, Clone)]
pub struct AnonymousRuleQuery {
    /// Term bindings — keys `this` (effect entity) and
    /// `definition` (rule definition as a JSON string).
    pub terms: Parameters,
}

impl AnonymousRuleQuery {
    /// Construct a new query from a parameter map.
    pub fn new(terms: Parameters) -> Self {
        Self { terms }
    }
}

/// Pull a constant entity out of a term — either from a constant
/// term or from a variable the upstream selection already bound.
/// Returns `None` if the term is unbound or absent. Mirrors the
/// `resolve_entity_filter` helper in [`crate::concept`].
fn resolve_entity_filter(term: &Option<Term<dialog_query::Any>>, input: &Match) -> Option<Entity> {
    let t = term.as_ref()?;
    match t {
        Term::Constant(value) => Entity::try_from(value.clone()).ok(),
        Term::Variable { name: Some(_), .. } => {
            let binding = input.lookup(t).ok()?;
            Entity::try_from(binding.as_value()?.clone()).ok()
        }
        Term::Variable { name: None, .. } => None,
    }
}

impl Application for AnonymousRuleQuery {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Scope<'a>,
    {
        let app = self;
        try_stream! {
            for await each in selection {
                let input = each?;

                let this_term = app.terms.get("this").cloned();
                let definition_term = app.terms.get("definition").cloned();

                let this_filter = resolve_entity_filter(&this_term, &input);

                // Enumerate every effect entity via the
                // `db.meta/effect = db:effect` marker. When
                // `this` is a constant the selection is already
                // narrowed to that entity.
                let marker = effect_marker_entity();
                let this_term_for_marker: Term<Entity> = match &this_filter {
                    Some(e) => Term::Constant(Value::Entity(e.clone())),
                    None => Term::var("__rule_query_this"),
                };
                let claims: Vec<dialog_query::Claim> = the!("db.meta/effect")
                    .of(this_term_for_marker)
                    .is(marker)
                    .perform(env)
                    .try_vec()
                    .await?;

                for claim in claims {
                    let entity = claim.of.clone();

                    // Rehydrate the effect's source + polarity
                    // straight off the same selection environment.
                    let Some(effect) = load_effect_facts(&entity, env).await? else {
                        continue;
                    };

                    let mut m = input.clone();
                    if let Some(ref t) = this_term {
                        m.bind(t, Value::Entity(entity.clone()))?;
                    }
                    if let Some(ref t) = definition_term {
                        let definition = RuleDefinition::from_effect(&effect);
                        let json = serde_json::to_string(&definition)
                            .map_err(|e| EvaluationError::Store(e.to_string()))?;
                        m.bind(t, Value::String(json))?;
                    }
                    yield m;
                }
            }
        }
    }

    fn realize(&self, source: Match) -> Result<Self::Conclusion, EvaluationError> {
        // `ConceptConclusion`'s fields are private; delegate to
        // dialog's `ConceptQuery::realize`, which only reads
        // `terms` and the match's `this` binding. The `predicate`
        // is unused by `realize` so a stub stands in.
        let synthetic = ConceptQuery {
            terms: self.terms.clone(),
            // A descriptor must have at least one required field;
            // `realize` never reads the predicate, so a single
            // placeholder field suffices.
            predicate: ConceptDescriptor::try_from(vec![(
                "_",
                dialog_query::AttributeDescriptor::new(
                    the!("db.rule/stub"),
                    "",
                    dialog_query::Cardinality::default(),
                    None,
                ),
            )])
            .expect("single-field stub descriptor is valid"),
        };
        Application::realize(&synthetic, source)
    }
}

/// Rehydrate an effect from its `db.effect/source` and
/// `db.effect/polarity` claims read straight off a query
/// selection environment.
///
/// Takes the raw `Provider<Select>` selection env that
/// [`Application::evaluate`] is handed, rather than a `&Branch`.
/// Returns `None` when the entity has no `source` claim (a
/// dangling marker).
async fn load_effect_facts<'a, Env>(
    entity: &Entity,
    env: &'a Env,
) -> Result<Option<Effect>, EvaluationError>
where
    Env: Scope<'a>,
{
    let source_claims: Vec<dialog_query::Claim> = the!("db.effect/source")
        .of(Term::<Entity>::from(entity.clone()))
        .is(Term::<String>::var("__rule_query_source"))
        .perform(env)
        .try_vec()
        .await?;
    let Some(source_claim) = source_claims.into_iter().next() else {
        return Ok(None);
    };
    let Value::String(source) = source_claim.is else {
        return Err(EvaluationError::Store(
            "db.effect/source claim was not a string".to_string(),
        ));
    };

    let polarity_claims: Vec<dialog_query::Claim> = the!("db.effect/polarity")
        .of(Term::<Entity>::from(entity.clone()))
        .is(Term::<String>::var("__rule_query_polarity"))
        .perform(env)
        .try_vec()
        .await?;
    let Some(polarity_claim) = polarity_claims.into_iter().next() else {
        return Err(EvaluationError::Store(
            "missing db.effect/polarity claim".to_string(),
        ));
    };
    let Value::String(polarity_str) = polarity_claim.is else {
        return Err(EvaluationError::Store(
            "db.effect/polarity claim was not a string".to_string(),
        ));
    };
    let polarity = EffectPolarity::parse(&polarity_str)
        .ok_or_else(|| EvaluationError::Store(format!("invalid polarity {polarity_str:?}")))?;

    let effect = Effect::from_source(&source, polarity)
        .map_err(|e| EvaluationError::Store(format!("effect rehydrate failed: {e}")))?;
    Ok(Some(effect))
}

/// The well-known descriptor for the "rule of rules" head.
///
/// Its `with` map names the marker (`db.meta/effect`) and the
/// synthesised `definition` field — enough for the analyzer to
/// project the fields a `rule:` head exposes. The `definition`
/// claim has no real EAV backing; it is produced only by
/// [`AnonymousRuleQuery::evaluate`], which the concept query
/// planner dispatches to whenever it sees this descriptor's
/// `this()`.
pub fn rule_of_rule_descriptor() -> &'static ConceptDescriptor {
    static DESCRIPTOR: std::sync::OnceLock<ConceptDescriptor> = std::sync::OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        serde_json::from_value(serde_json::json!({
            "description": "Every inductive rule installed on a branch.",
            "with": {
                "effect":     { "the": "db.meta/effect",     "as": "Entity", "cardinality": "one" },
                "definition": { "the": "db.effect/source",   "as": "Text",   "cardinality": "one" }
            }
        }))
        .expect("rule-of-rule descriptor is well-formed")
    })
}

#[cfg(test)]
mod tests {
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
    use dialog_query::{InductiveRule, Proposition};

    use crate::effect::EFFECT_SYSTEM;
    use crate::rule::Rule;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// `counter` concept with a single `count` field.
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

    /// `increment` command concept.
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

    /// Install an effect on a branch, then query `rule:` via
    /// [`AnonymousRuleQuery`] and assert it surfaces the rule with
    /// its definition. The `definition` field round-trips to a
    /// [`RuleDefinition`] whose `rule` descriptor and `polarity`
    /// match the installed effect — the rule-side parallel of
    /// `concept.rs`'s `it_returns_concept_with_source_from_concept_query`.
    #[dialog_common::test]
    async fn it_enumerates_installed_rules_via_rule_query() -> anyhow::Result<()> {
        use dialog_operator::helpers::{test_operator_with_profile, test_repo};
        use dialog_query::{Any, Output as _, Parameters, Term};

        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        // Install one assert-polarity effect on the branch.
        let rule = InductiveRule::new(counter_head(), increment_body()).expect("rule compiles");
        let effect = Effect::asserting(rule);
        let effect_entity = effect.this();
        let expected = effect.descriptor();
        branch
            .transaction()
            .assert(Rule::asserting(effect.clone()))
            .commit()
            .perform(&operator)
            .await?;

        // Query `rule:` with `this` and `definition` as variables.
        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::<Any>::var("this"));
        terms.insert("definition".to_string(), Term::<Any>::var("definition"));

        let conclusions: Vec<ConceptConclusion> = branch
            .query()
            .select(AnonymousRuleQuery::new(terms))
            .perform(&operator)
            .try_vec()
            .await?;

        // Exactly one rule on the branch — the one we installed.
        assert_eq!(
            conclusions.len(),
            1,
            "expected one rule row; saw {conclusions:?}"
        );
        let row = &conclusions[0];

        // `this` binds the effect entity.
        let this: Entity =
            Entity::try_from(row.source().lookup(&Term::<Any>::var("this"))?.content()?)
                .expect("this binding must be an entity");
        assert_eq!(this, effect_entity);

        // `definition` carries the JSON-serialised RuleDefinition.
        let definition_json: String = String::try_from(
            row.source()
                .lookup(&Term::<Any>::var("definition"))?
                .content()?,
        )
        .expect("definition binding must be a string");
        let definition: RuleDefinition = serde_json::from_str(&definition_json)?;
        assert_eq!(definition.polarity, EffectPolarity::Assert);
        assert_eq!(
            definition.rule.assert.as_ref().map(|head| head.this()),
            expected.assert.as_ref().map(|head| head.this())
        );
        assert_eq!(definition.rule.when.len(), expected.when.len());
        assert_eq!(definition.rule.unless.len(), expected.unless.len());
        Ok(())
    }
}
