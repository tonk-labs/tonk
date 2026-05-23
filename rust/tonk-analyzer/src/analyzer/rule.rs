//! Rule-side analysis — lifts a `rule!:` claim's body into a
//! compiled [`tonk_core::effect::Effect`].
//!
//! `rule!:` is structurally a [`tonk_notation::Expression::Claim`]
//! over the built-in `rule` concept; the analyzer's mutation pass
//! recognises the predicate name and dispatches to [`lift_rule`].
//!
//! The lift extracts the assert!:/retract!: polarity, the head
//! concept name (the value of that field), the `when:` and
//! `unless:` premise lists (carried as
//! [`FieldValue::Premises`][tonk_notation::FieldValue::Premises]),
//! resolves each premise's concept against the scope, translates
//! each premise's `where:` bindings into dialog `Term`s, and
//! finally compiles the [`InductiveRule`] via dialog's planner.
//!
//! Resolution and validation happen here; the install-time
//! transient-trigger check runs separately at the evaluator's
//! commit step where a branch is available.

use dialog_query::concept::query::ConceptQuery;
use dialog_query::formula::query::FormulaQuery;
use dialog_query::premise::Premise as DialogPremise;
use dialog_query::{InductiveRule, Negation, Parameters, Proposition, Term};
use tonk_notation::{
    Application as SyntaxApplication, FieldValue, Premise as NotationPremise, Scalar,
};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::field_value_to_term;
use super::formula::{FormulaInfo, lookup_formula};
use super::resolver::Resolver;
use super::scope::Scope;
use crate::analyzer::Working;
use dialog_artifacts::Entity;
use tonk_core::effect::{Effect, EffectPolarity};

/// Outcome of inspecting a `rule!:` claim body — install (lift to
/// an [`Effect`]) or retract (point at an existing rule entity).
///
/// The two shapes live in the same notation:
///
/// - Install: `rule!: assert!: <head>, when: [...]` — body carries
///   polarity + premises, no `..: _`.
/// - Retract: `rule!: this: effect:<entity>, ..: _` — body carries
///   the rule's entity in `this:` and `..: _` as the
///   retract-everything sentinel.
pub(crate) enum RuleAction {
    /// Install a new rule. The carried [`Effect`] is the lifted,
    /// dialog-planner-validated rule.
    Install(Effect),
    /// Retract an installed rule. The carried entity URI names the
    /// effect on the branch whose `dialog.effect/*` facts the
    /// evaluator should dissociate.
    Retract(Entity),
}

/// `true` when the claim body is the rule-retract shape
/// (`this: <entity>` plus `..: _` and no `assert!:` / `retract!:`).
///
/// `..: _` is the syntactic marker that says "retract every
/// attribute of this entity"; combined with `this:` it identifies a
/// specific rule. Used by the analyzer to pick between
/// [`RuleAction::Install`] and [`RuleAction::Retract`].
fn is_rule_retract_body(application: &SyntaxApplication) -> bool {
    let has_rest_retract = application
        .fields
        .iter()
        .any(|f| f.name == ".." && matches!(f.value, FieldValue::Blank));
    let has_polarity = application
        .fields
        .iter()
        .any(|f| f.name == "assert!" || f.name == "retract!");
    has_rest_retract && !has_polarity
}

/// Dispatch a `rule!:` claim to install or retract, depending on
/// the body shape. The analyzer calls this once per `rule!:` claim
/// it encounters.
pub(crate) async fn lift_rule_claim<R: Resolver + ?Sized>(
    application: &SyntaxApplication,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<RuleAction, AnalyzeError> {
    if is_rule_retract_body(application) {
        let entity = parse_rule_retract_target(application)?;
        Ok(RuleAction::Retract(entity))
    } else {
        let effect = lift_rule(application, scope, analysis).await?;
        Ok(RuleAction::Install(effect))
    }
}

/// Read the `this:` field out of a rule-retract body as the effect
/// entity URI. Rejects missing `this:` or non-URI `this:` values
/// with a diagnostic.
fn parse_rule_retract_target(application: &SyntaxApplication) -> Result<Entity, AnalyzeError> {
    let this = application
        .fields
        .iter()
        .find(|f| f.name == "this")
        .ok_or_else(|| {
            AnalyzeError::at(
                AnalyzeErrorKind::RuleCompileFailed {
                    reason: "rule retraction (`rule!: this: <entity> ..: _`) must name a `this:` \
                             field carrying the effect entity URI"
                        .into(),
                },
                application.range,
            )
        })?;
    let uri = match &this.value {
        FieldValue::Uri(uri) => uri.clone(),
        _ => {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnsupportedFieldValue {
                    field: "this".into(),
                    form: "effect entity URI (e.g. `effect:<base58>`)",
                },
                this.value_range,
            ));
        }
    };
    uri.parse()
        .map_err(|e: dialog_artifacts::DialogArtifactsError| {
            AnalyzeError::at(
                AnalyzeErrorKind::InvalidSubjectUri {
                    subject: uri,
                    reason: e.to_string(),
                },
                this.value_range,
            )
        })
}

/// Lift a `rule!:` claim's body into an [`Effect`] ready to
/// install.
///
/// `application` is the parsed body of the `rule!:` claim — the
/// fields list carries the polarity (`assert!:` or `retract!:`),
/// optional `description:`, and `when:` / `unless:` premise lists.
/// Validation that used to live in the parser (exactly one
/// polarity, non-empty `when:`) lives here so each diagnostic
/// can point at semantically meaningful ranges.
pub(crate) async fn lift_rule<R: Resolver + ?Sized>(
    application: &SyntaxApplication,
    scope: &Scope<'_, R>,
    analysis: &Working,
) -> Result<Effect, AnalyzeError> {
    let body = parse_rule_body(application)?;

    // ---- Head concept ----
    let head_descriptor = {
        let name = body.conclusion.as_str();
        let resolved = scope
            .resolve_concept(name)
            .await
            .map_err(|e| {
                AnalyzeError::at(
                    AnalyzeErrorKind::ResolverFailed {
                        context: format!("rule head concept {name:?}"),
                        reason: e.to_string(),
                    },
                    body.conclusion_range,
                )
            })?
            .ok_or_else(|| {
                AnalyzeError::at(
                    AnalyzeErrorKind::UnknownConcept { name: name.into() },
                    body.conclusion_range,
                )
            })?;
        resolved.descriptor.concept().clone()
    };

    // ---- Premises ----
    let mut dialog_premises: Vec<DialogPremise> = Vec::new();
    for premise in body.when {
        let proposition = lift_premise(premise, scope, analysis).await?;
        dialog_premises.push(DialogPremise::Assert(proposition));
    }
    for premise in body.unless {
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
            application.range,
        )
    })?;

    Ok(Effect::new(inductive, body.polarity))
}

/// Extracted shape of a `rule!:` claim body. Built by walking the
/// claim's fields once and pulling out the rule-specific slots
/// (polarity + conclusion, `when:`, `unless:`, `description:`).
/// Doing this in one place keeps [`lift_rule`] free of grammar
/// checks.
struct RuleBody<'a> {
    polarity: EffectPolarity,
    conclusion: String,
    conclusion_range: lsp_types::Range,
    when: Vec<&'a NotationPremise>,
    unless: Vec<&'a NotationPremise>,
}

/// Walk a `rule!:` claim's fields and extract the typed rule
/// pieces, reporting shape errors with semantic context.
///
/// Validation:
///
/// - Exactly one of `assert!:` / `retract!:` (more is an error,
///   none is an error).
/// - `when:` is required and non-empty.
/// - `description:` (when present) must be a string literal.
/// - Unknown body keys raise a diagnostic.
fn parse_rule_body(application: &SyntaxApplication) -> Result<RuleBody<'_>, AnalyzeError> {
    let mut polarity: Option<(EffectPolarity, String, lsp_types::Range, lsp_types::Range)> = None;
    let mut when: Option<(Vec<&NotationPremise>, lsp_types::Range)> = None;
    let mut unless: Vec<&NotationPremise> = Vec::new();

    for field in &application.fields {
        match field.name.as_str() {
            "assert!" | "retract!" => {
                let new_polarity = if field.name == "assert!" {
                    EffectPolarity::Assert
                } else {
                    EffectPolarity::Retract
                };
                if let Some((_, _, prior_range, _)) = polarity {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::RuleCompileFailed {
                            reason: format!(
                                "rule already declared polarity at \
                                 {}:{}, a rule has exactly one polarity",
                                prior_range.start.line + 1,
                                prior_range.start.character + 1,
                            ),
                        },
                        field.name_range,
                    ));
                }
                let concept = match &field.value {
                    FieldValue::Symbol(s) => s.clone(),
                    FieldValue::Literal(Scalar::String(s)) => s.clone(),
                    _ => {
                        return Err(AnalyzeError::at(
                            AnalyzeErrorKind::UnsupportedFieldValue {
                                field: field.name.clone(),
                                form: "concept name (bare symbol or string literal)",
                            },
                            field.value_range,
                        ));
                    }
                };
                polarity = Some((new_polarity, concept, field.name_range, field.value_range));
            }
            "when" => {
                if when.is_some() {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::RuleCompileFailed {
                            reason: "rule already declared `when:`, combine premises into one list"
                                .into(),
                        },
                        field.name_range,
                    ));
                }
                let FieldValue::Premises(items) = &field.value else {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::UnsupportedFieldValue {
                            field: "when".into(),
                            form: "a list of `{assert: <concept>, where: {...}}` premises",
                        },
                        field.value_range,
                    ));
                };
                when = Some((items.iter().collect(), field.value_range));
            }
            "unless" => {
                if !unless.is_empty() {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::RuleCompileFailed {
                            reason: "rule already declared `unless:`, combine premises into one \
                                     list"
                                .into(),
                        },
                        field.name_range,
                    ));
                }
                let FieldValue::Premises(items) = &field.value else {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::UnsupportedFieldValue {
                            field: "unless".into(),
                            form: "a list of `{assert: <concept>, where: {...}}` premises",
                        },
                        field.value_range,
                    ));
                };
                unless = items.iter().collect();
            }
            "description" => {
                // Description is preserved on the descriptor through
                // dialog's planner; we don't model it on the analyzer
                // side beyond shape validation.
                if !matches!(&field.value, FieldValue::Literal(Scalar::String(_))) {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::UnsupportedFieldValue {
                            field: "description".into(),
                            form: "quoted string literal",
                        },
                        field.value_range,
                    ));
                }
            }
            "this" | ".." => {
                // Reserved meta-keys. `this:` selects a target rule
                // entity for retraction (`rule!: this: effect:abc
                // ..: _`); `..: _` is the retract-all sentinel. The
                // outer claim flow handles these; the rule lift
                // skips them.
                continue;
            }
            other => {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::RuleCompileFailed {
                        reason: format!(
                            "unknown `rule!:` body key `{other}` (valid keys: assert!, retract!, \
                             when, unless, description)"
                        ),
                    },
                    field.name_range,
                ));
            }
        }
    }

    let (polarity, conclusion, _, conclusion_range) = polarity.ok_or_else(|| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: "rule must declare `assert!:` or `retract!:` with a head concept name"
                    .into(),
            },
            application.range,
        )
    })?;

    let (when, when_range) = when.ok_or_else(|| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: "rule must declare `when:` with at least one premise".into(),
            },
            application.range,
        )
    })?;
    if when.is_empty() {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: "rule's `when:` must list at least one premise".into(),
            },
            when_range,
        ));
    }

    Ok(RuleBody {
        polarity,
        conclusion,
        conclusion_range,
        when,
        unless,
    })
}

/// Resolve one notation premise into a dialog [`Proposition`].
///
/// A premise head names either a built-in formula (`math/sum`,
/// `boolean/and`, …) or a concept. Formula names are recognised
/// first — they're a fixed set that never lives on the branch, so
/// the registry lookup is authoritative. Anything else resolves
/// as a concept.
async fn lift_premise<R: Resolver + ?Sized>(
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
async fn lift_formula_premise<R: Resolver + ?Sized>(
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
