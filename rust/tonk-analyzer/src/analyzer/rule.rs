//! Rule-side analysis — lifts a parsed
//! [`tonk_notation::Rule`] into a compiled
//! [`tonk_core::effect::Effect`].
//!
//! The lift resolves the head concept and each premise's
//! concept against the in-doc scope + branch, translates each
//! premise's `where:` bindings into dialog `Term`s, builds the
//! list of dialog [`Premise`]s, and finally compiles the
//! [`InductiveRule`] via dialog's planner. The compiled rule is
//! paired with the parsed [`RulePolarity`] to produce an
//! [`Effect`].
//!
//! Resolution and validation happen here; the install-time
//! transient-trigger check runs separately at the evaluator's
//! commit step where a branch is available.

use dialog_artifacts::Entity;
use dialog_query::concept::query::ConceptQuery;
use dialog_query::formula::query::FormulaQuery;
use dialog_query::premise::Premise as DialogPremise;
use dialog_query::{InductiveRule, Negation, Parameters, Proposition, Term};
use tonk_notation::{Premise as NotationPremise, Rule, RuleInstall, RulePolarity, RuleRetract};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::field_value_to_term;
use super::formula::{FormulaInfo, lookup_formula};
use super::resolver::Resolver;
use super::scope::Scope;
use crate::analysis::RuleAnalysis;
use crate::analyzer::Working;
use tonk_core::effect::{Effect, EffectPolarity};

/// Lift a parsed [`Rule`] into a [`RuleAnalysis`].
///
/// A `rule!:` carries one of two shapes:
///
/// - [`Rule::Install`] lifts into a compiled [`Effect`]
///   ([`RuleAnalysis::Install`]).
/// - [`Rule::Retract`] lifts into the effect entity to uninstall
///   ([`RuleAnalysis::Retract`]).
pub(crate) async fn lift_rule<R: Resolver>(
    rule: &Rule,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<RuleAnalysis, AnalyzeError> {
    match rule {
        Rule::Install(install) => {
            let effect = lift_rule_install(install, scope, analysis).await?;
            Ok(RuleAnalysis::Install(effect))
        }
        Rule::Retract(retract) => Ok(RuleAnalysis::Retract(lift_rule_retract(retract)?)),
    }
}

/// Resolve a [`Rule::Retract`]'s `this:` URI into the effect
/// entity to uninstall. The parser already constrained `this:`
/// to a URI form; this re-parses it into a typed [`Entity`].
fn lift_rule_retract(retract: &RuleRetract) -> Result<Entity, AnalyzeError> {
    retract.entity.value.parse::<Entity>().map_err(|e| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: format!(
                    "rule retraction `this:` {:?} is not a valid entity URI: {e}",
                    retract.entity.value
                ),
            },
            retract.entity.range,
        )
    })
}

/// Lift a parsed [`RuleInstall`] into an [`Effect`] ready to
/// install.
///
/// Each premise's concept and the head concept are resolved
/// through `scope`; missing names surface as
/// [`AnalyzeErrorKind::UnknownConcept`]. Each premise's
/// `where:` field is translated through the existing
/// [`field_value_to_term`] helper, so `?var` / literal /
/// symbol / blank forms behave consistently with the rest of
/// the analyzer.
async fn lift_rule_install<R: Resolver>(
    rule: &RuleInstall,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<Effect, AnalyzeError> {
    // ---- Head concept ----
    let head_descriptor = {
        let name = rule.conclusion.value.as_str();
        let resolved = scope
            .resolve_concept(name)
            .await
            .map_err(|e| {
                AnalyzeError::at(
                    AnalyzeErrorKind::ResolverFailed {
                        context: format!("rule head concept {name:?}"),
                        reason: e.to_string(),
                    },
                    rule.conclusion.range,
                )
            })?
            .ok_or_else(|| {
                AnalyzeError::at(
                    AnalyzeErrorKind::UnknownConcept { name: name.into() },
                    rule.conclusion.range,
                )
            })?;
        resolved.descriptor.concept().clone()
    };

    // ---- Premises ----
    let mut dialog_premises: Vec<DialogPremise> = Vec::new();
    for premise in &rule.when {
        let proposition = lift_premise(premise, scope, analysis).await?;
        dialog_premises.push(DialogPremise::Assert(proposition));
    }
    for premise in &rule.unless {
        let proposition = lift_premise(premise, scope, analysis).await?;
        dialog_premises.push(DialogPremise::Unless(Negation(proposition)));
    }

    // ---- Compile through dialog's planner ----
    // Catches unbound-head-variable, no-unsatisfiable-premise,
    // and similar structural issues.
    let inductive = InductiveRule::new(head_descriptor, dialog_premises).map_err(|e| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: e.to_string(),
            },
            rule.range,
        )
    })?;

    let polarity = match rule.polarity {
        RulePolarity::Assert => EffectPolarity::Assert,
        RulePolarity::Retract => EffectPolarity::Retract,
    };
    Ok(Effect::new(inductive, polarity))
}

/// Resolve one notation premise into a dialog [`Proposition`].
///
/// A premise head names either a built-in formula (`math/sum`,
/// `boolean/and`, …) or a concept. Formula names are recognised
/// first — they're a fixed set that never lives on the branch, so
/// the registry lookup is authoritative. Anything else resolves
/// as a concept.
async fn lift_premise<R: Resolver>(
    premise: &NotationPremise,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<Proposition, AnalyzeError> {
    let name = premise.concept.value.as_str();

    if let Some(formula) = lookup_formula(name) {
        return lift_formula_premise(premise, formula, scope, analysis).await;
    }

    let resolved = scope
        .resolve_concept(name)
        .await
        .map_err(|e| {
            AnalyzeError::at(
                AnalyzeErrorKind::ResolverFailed {
                    context: format!("rule premise concept {name:?}"),
                    reason: e.to_string(),
                },
                premise.concept.range,
            )
        })?
        .ok_or_else(|| {
            AnalyzeError::at(
                AnalyzeErrorKind::UnknownConcept { name: name.into() },
                premise.concept.range,
            )
        })?;
    let descriptor = resolved.descriptor.concept().clone();

    // Build the term map for this premise's `where:` bindings.
    // Every operand of the head concept must be present (either
    // bound by the user or auto-filled with an anonymous
    // variable so the engine has a slot for it).
    let mut terms = Parameters::new();

    // `this` slot: explicit user binding wins; otherwise mint a
    // unique anonymous variable so the premise can still match.
    let user_this = premise.bindings.iter().find(|f| f.name == "this");
    if let Some(field) = user_this {
        let term = field_value_to_term(
            "this",
            &field.value,
            field.value_range,
            scope,
            analysis,
            None,
        )
        .await?;
        terms.insert("this".into(), term);
    } else {
        terms.insert("this".into(), Term::<dialog_query::Any>::unique());
    }

    // Per-field bindings declared by the user. Fields not
    // mentioned default to anonymous variables (consistent with
    // how `Query` bodies project unmentioned fields).
    for (field_name, attr) in descriptor.with().iter() {
        if field_name == "this" {
            continue;
        }
        let user_binding = premise.bindings.iter().find(|f| f.name == *field_name);
        let term = match user_binding {
            Some(field) => {
                field_value_to_term(
                    field_name,
                    &field.value,
                    field.value_range,
                    scope,
                    analysis,
                    attr.content_type(),
                )
                .await?
            }
            None => Term::<dialog_query::Any>::unique(),
        };
        terms.insert(field_name.to_string(), term);
    }

    // Reject premise bindings naming fields the concept doesn't
    // have — same shape as the query-side check.
    for field in &premise.bindings {
        if field.name == "this" {
            continue;
        }
        if !descriptor.with().iter().any(|(n, _)| n == field.name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnknownField {
                    concept: name.into(),
                    field: field.name.clone(),
                },
                field.name_range,
            ));
        }
    }

    Ok(Proposition::Concept(ConceptQuery {
        terms,
        predicate: descriptor,
    }))
}

/// Lift a premise whose head names a built-in formula into a
/// dialog [`Proposition::Formula`].
///
/// Unlike concepts, a formula's operand set is fixed and known
/// up front (from its [`FormulaInfo`] cells), so validation is
/// strict in both directions:
///
/// - An operand the user wrote that the formula doesn't have is
///   an [`AnalyzeErrorKind::UnknownFormulaOperand`].
/// - A required input operand the user *didn't* write is a
///   [`AnalyzeErrorKind::MissingFormulaOperand`] — formulas can't
///   compute without their inputs, so unlike concept fields these
///   don't get auto-filled with anonymous variables.
///
/// Optional (`#[output]`) operands the user omits are filled with
/// a unique anonymous variable: the formula still computes the
/// value, it just isn't joined anywhere.
async fn lift_formula_premise<R: Resolver>(
    premise: &NotationPremise,
    formula: FormulaInfo,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<Proposition, AnalyzeError> {
    // Reject `where:` operands the formula doesn't declare. Done
    // first so an obvious typo surfaces before missing-operand
    // noise.
    for field in &premise.bindings {
        if !formula.operands().any(|operand| operand == field.name) {
            let mut valid: Vec<&str> = formula.operands().collect();
            valid.sort_unstable();
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnknownFormulaOperand {
                    formula: formula.name.to_owned(),
                    operand: field.name.clone(),
                    valid: valid.join(", "),
                },
                field.name_range,
            ));
        }
    }

    // Translate every operand the formula declares. Required
    // operands must be bound; optional/output operands default
    // to a fresh anonymous variable.
    let mut terms = Parameters::new();
    for (operand, cell) in formula.cells.iter() {
        let term = match premise.bindings.iter().find(|f| f.name == operand) {
            Some(field) => {
                field_value_to_term(
                    operand,
                    &field.value,
                    field.value_range,
                    scope,
                    analysis,
                    None,
                )
                .await?
            }
            None if cell.requirement().is_required() => {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::MissingFormulaOperand {
                        formula: formula.name.to_owned(),
                        operand: operand.to_owned(),
                    },
                    premise.concept.range,
                ));
            }
            None => Term::<dialog_query::Any>::unique(),
        };
        terms.insert(operand.to_string(), term);
    }

    // Construct the typed `FormulaQuery` through its name-keyed
    // serde representation: `{"assert": <name>, "where": <terms>}`.
    // dialog-query owns the formula↔name mapping, so routing
    // through serde keeps the analyzer from having to match every
    // formula type by hand.
    let value = serde_json::json!({ "assert": formula.name, "where": terms });
    let query: FormulaQuery = serde_json::from_value(value).map_err(|e| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: format!("formula {:?}: {e}", formula.name),
            },
            premise.concept.range,
        )
    })?;

    Ok(Proposition::Formula(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dialog_artifacts::Entity;
    use dialog_query::AttributeDescriptor;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use std::collections::HashMap;
    use tonk_core::mutation::ConceptDescriptor as DurableConceptDescriptor;
    use tonk_notation::parse;
    use tonk_schema::resolution::{AttributeDefinition, ConceptDefinition, ResolveError};

    /// Resolver backed by an in-memory map of concept name →
    /// descriptor. Lets the test feed pre-built concepts to the
    /// lifter without standing up a branch.
    struct TestResolver {
        concepts: HashMap<String, ConceptDefinition>,
    }

    impl TestResolver {
        fn new() -> Self {
            Self {
                concepts: HashMap::new(),
            }
        }

        fn declare(&mut self, name: &str, descriptor: ConceptDescriptor) {
            let entity = descriptor.this();
            self.concepts.insert(
                name.to_string(),
                ConceptDefinition {
                    entity,
                    descriptor: DurableConceptDescriptor::Durable(descriptor),
                },
            );
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl crate::analyzer::resolver::Resolver for TestResolver {
        async fn resolve_concept(
            &self,
            name: &str,
        ) -> Result<Option<ConceptDefinition>, ResolveError> {
            Ok(self.concepts.get(name).cloned())
        }
        async fn resolve_attribute(
            &self,
            _name: &str,
        ) -> Result<Option<AttributeDefinition>, ResolveError> {
            Ok(None)
        }
        async fn resolve_attribute_by_entity(
            &self,
            _entity: &Entity,
        ) -> Result<Option<AttributeDefinition>, ResolveError> {
            Ok(None)
        }
        async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolveError> {
            Ok(None)
        }
    }

    /// Build a `ConceptDescriptor` with one cardinality-one
    /// string field. Same helper shape as the effects-side
    /// tests use.
    fn one_text_field(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::String),
            ),
        )])
    }

    /// Extract the lone `Statement::InstallEffect` from an
    /// analysis tree, panicking if the document does not lower to
    /// exactly one statement and it is not an effect install.
    fn only_installed_effect(
        tree: &crate::analysis::Analysis<tonk_notation::Syntax>,
    ) -> tonk_core::effect::Effect {
        let statements = tree.analysis.statements();
        assert_eq!(statements.len(), 1);
        match &statements[0].statement {
            tonk_core::transact::Statement::InstallEffect(effect) => effect.clone(),
            other => panic!("expected Statement::InstallEffect, got {other:?}"),
        }
    }

    /// End-to-end happy path: a `rule!:` with one positive
    /// `when` premise lifts into a compiled `Effect` and lands
    /// in `analysis.mutate.statements` as a `Statement::InstallEffect`.
    #[dialog_common::test]
    async fn it_lifts_a_rule_into_an_effect() {
        let mut resolver = TestResolver::new();
        resolver.declare("ping", one_text_field("io.gozala.ping", "tag"));
        resolver.declare("pong", one_text_field("io.gozala.pong", "tag"));

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("parsed syntax");

        let analysis = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect("analyze should succeed");

        let effect = only_installed_effect(&analysis);
        assert_eq!(effect.polarity(), EffectPolarity::Assert);
        // Head concept entity matches the pong descriptor.
        assert_eq!(
            effect.conclusion(),
            one_text_field("io.gozala.pong", "tag").this()
        );
        // Reverse-index keys cover the body's attribute.
        let on_uris: Vec<String> = effect
            .on_entities()
            .into_iter()
            .map(|e| e.to_string())
            .collect();
        assert!(
            on_uris.iter().any(|u| u == "on:io.gozala.ping/tag"),
            "expected on:io.gozala.ping/tag in {on_uris:?}"
        );
    }

    /// Retract-polarity head lifts to `EffectPolarity::Retract`.
    #[dialog_common::test]
    async fn it_lifts_retract_polarity() {
        let mut resolver = TestResolver::new();
        resolver.declare("ack", one_text_field("io.gozala.mailbox", "target"));
        resolver.declare("message", one_text_field("io.gozala.mailbox", "body"));

        let doc = "\
rule!:\n\
\x20 retract!: message\n\
\x20 when:\n\
\x20   - assert: ack\n\
\x20     where: { target: ?this }\n\
\x20   - assert: message\n\
\x20     where: { this: ?this, body: ?body }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let analysis = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect("analyze should succeed");

        assert_eq!(
            only_installed_effect(&analysis).polarity(),
            EffectPolarity::Retract
        );
    }

    /// A `rule!:` retraction (`this: <uri> / ..: _`) lifts to a
    /// trailing `Statement::RetractEffect` carrying the effect
    /// entity to uninstall.
    #[dialog_common::test]
    async fn it_lifts_a_rule_retraction() {
        let resolver = TestResolver::new();
        let doc = "\
rule!:\n\
\x20 this: effect:7Egd23og28aqm1dkPbyQBE6YZXbNDWraiggU2Uq7rwj8\n\
\x20 ..: _\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("parsed syntax");
        let analysis = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect("analyze should succeed");

        let statements = analysis.analysis.statements();
        assert_eq!(statements.len(), 1);
        let expected: Entity = "effect:7Egd23og28aqm1dkPbyQBE6YZXbNDWraiggU2Uq7rwj8"
            .parse()
            .unwrap();
        match &statements[0].statement {
            tonk_core::transact::Statement::RetractEffect(entity) => {
                assert_eq!(*entity, expected);
            }
            other => panic!("expected Statement::RetractEffect, got {other:?}"),
        }
    }

    /// Unknown head concept name surfaces as
    /// [`AnalyzeErrorKind::UnknownConcept`].
    #[dialog_common::test]
    async fn it_rejects_unknown_head_concept() {
        let mut resolver = TestResolver::new();
        resolver.declare("ping", one_text_field("io.gozala.ping", "tag"));

        let doc = "\
rule!:\n\
\x20 assert!: missing-concept\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: {}\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownConcept { ref name } if name == "missing-concept"
            ),
            "expected UnknownConcept(missing-concept), got {err:?}"
        );
    }

    /// Unknown premise concept name also rejects.
    #[dialog_common::test]
    async fn it_rejects_unknown_premise_concept() {
        let mut resolver = TestResolver::new();
        resolver.declare("pong", one_text_field("io.gozala.pong", "tag"));

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: missing-premise\n\
\x20     where: {}\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownConcept { ref name } if name == "missing-premise"
            ),
            "expected UnknownConcept(missing-premise), got {err:?}"
        );
    }

    /// Premise binding naming a field the concept doesn't have
    /// surfaces as [`AnalyzeErrorKind::UnknownField`].
    #[dialog_common::test]
    async fn it_rejects_unknown_premise_field() {
        let mut resolver = TestResolver::new();
        resolver.declare("ping", one_text_field("io.gozala.ping", "tag"));
        resolver.declare("pong", one_text_field("io.gozala.pong", "tag"));

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { wrong-field: ?tag }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownField { ref concept, ref field }
                    if concept == "ping" && field == "wrong-field"
            ),
            "expected UnknownField(ping, wrong-field), got {err:?}"
        );
    }

    /// Build a `ConceptDescriptor` with one cardinality-one
    /// unsigned-integer field — counters and other numeric
    /// concepts the formula tests assert against.
    fn one_uint_field(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::UnsignedInt),
            ),
        )])
    }

    /// A `rule!:` whose body uses the `math/sum` formula lifts
    /// into an effect — the counter-increment shape from the
    /// feature request.
    #[dialog_common::test]
    async fn it_lifts_a_rule_with_a_formula_premise() {
        let mut resolver = TestResolver::new();
        resolver.declare("counter", one_uint_field("io.gozala.counter", "count"));
        resolver.declare("increment", one_uint_field("io.gozala.increment", "by"));

        let doc = "\
rule!:\n\
\x20 assert!: counter\n\
\x20 when:\n\
\x20   - assert: counter\n\
\x20     where: { this: ?this, count: ?value }\n\
\x20   - assert: increment\n\
\x20     where: { this: ?this, by: 1 }\n\
\x20   - assert: math/sum\n\
\x20     where: { of: ?value, with: 1, is: ?count }\n";
        let parsed = parse(doc);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected parse diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("parsed syntax");
        let analysis = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect("analyze should succeed");
        assert_eq!(
            only_installed_effect(&analysis).polarity(),
            EffectPolarity::Assert
        );
    }

    /// A `where:` operand the formula doesn't have surfaces as
    /// [`AnalyzeErrorKind::UnknownFormulaOperand`].
    #[dialog_common::test]
    async fn it_rejects_unknown_formula_operand() {
        let mut resolver = TestResolver::new();
        resolver.declare("counter", one_uint_field("io.gozala.counter", "count"));

        let doc = "\
rule!:\n\
\x20 assert!: counter\n\
\x20 when:\n\
\x20   - assert: counter\n\
\x20     where: { this: ?this, count: ?value }\n\
\x20   - assert: math/sum\n\
\x20     where: { of: ?value, plus: 1, is: ?count }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownFormulaOperand { ref formula, ref operand, .. }
                    if formula == "math/sum" && operand == "plus"
            ),
            "expected UnknownFormulaOperand(math/sum, plus), got {err:?}"
        );
    }

    /// Omitting a required input operand surfaces as
    /// [`AnalyzeErrorKind::MissingFormulaOperand`].
    #[dialog_common::test]
    async fn it_rejects_missing_formula_operand() {
        let mut resolver = TestResolver::new();
        resolver.declare("counter", one_uint_field("io.gozala.counter", "count"));

        let doc = "\
rule!:\n\
\x20 assert!: counter\n\
\x20 when:\n\
\x20   - assert: counter\n\
\x20     where: { this: ?this, count: ?value }\n\
\x20   - assert: math/sum\n\
\x20     where: { of: ?value, is: ?count }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = crate::analyzer::analyze(&syntax, &resolver)
            .await
            .expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::MissingFormulaOperand { ref formula, ref operand }
                    if formula == "math/sum" && operand == "with"
            ),
            "expected MissingFormulaOperand(math/sum, with), got {err:?}"
        );
    }
}
