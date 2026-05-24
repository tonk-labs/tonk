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
use super::scope::Scope;
use crate::analyzer::Working;
use dialog_artifacts::Entity;
use tonk_core::effect::{Effect, EffectPolarity};
use tonk_schema::concept::QueryEnv;

/// Outcome of inspecting a `rule!:` claim body — install (lift to
/// an [`Effect`]) or retract (point at an existing rule entity).
///
/// The two shapes live in the same notation:
///
/// - Install: `rule!: assert!: <head>, when: [...]` — body carries
///   polarity + premises, no `..: _`. A `this: <entity>` may
///   additionally pin the install at a user-chosen entity.
/// - Retract: `rule!: this: effect:<entity>, ..: _` — body carries
///   the rule's entity in `this:` and `..: _` as the
///   retract-everything sentinel.
pub(crate) enum RuleAction {
    /// Install a new rule. The carried [`Effect`] is the lifted,
    /// dialog-planner-validated rule. `this` carries a user-chosen
    /// install-at entity when the body had `this: <entity>`;
    /// otherwise the install lands at [`Effect::this`].
    ///
    /// `effect` is boxed because [`InductiveRule`] dominates the
    /// variant's size; the unboxed shape would push `RuleAction`
    /// far past `Retract(Entity)` and trip `clippy::large_enum_variant`.
    /// The value is destructured immediately after construction, so
    /// the heap hop is paid once per `rule!:` claim.
    Install {
        /// The compiled rule body.
        effect: Box<Effect>,
        /// Caller-supplied install-at entity from `this: <entity>`.
        this: Option<Entity>,
    },
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
pub(crate) async fn lift_rule_claim<Env: QueryEnv>(
    application: &SyntaxApplication,
    scope: &Scope<'_>,
    env: &Env,
    analysis: &Working,
) -> Result<RuleAction, AnalyzeError> {
    if is_rule_retract_body(application) {
        let entity = parse_rule_this_entity(application)?.ok_or_else(|| {
            AnalyzeError::at(
                AnalyzeErrorKind::RuleCompileFailed {
                    reason: "rule retraction (`rule!: this: <entity> ..: _`) must name a `this:` \
                             field carrying the effect entity URI"
                        .into(),
                },
                application.range,
            )
        })?;
        Ok(RuleAction::Retract(entity))
    } else {
        let this = parse_rule_this_entity(application)?;
        let effect = lift_rule(application, scope, env, analysis).await?;
        Ok(RuleAction::Install {
            effect: Box::new(effect),
            this,
        })
    }
}

/// Read an optional `this:` URI field out of a `rule!:` body.
///
/// Returns `Ok(None)` when no `this:` field is present. Returns
/// `Ok(Some(entity))` when the field carries a parseable URI.
/// Errors when the field is present but holds a non-URI value
/// (variable, blank, literal, …) — `rule!:` is a "named install"
/// or "named retract," not a templated one, so a `this: ?var` would
/// be ambiguous about which entity to write at.
fn parse_rule_this_entity(application: &SyntaxApplication) -> Result<Option<Entity>, AnalyzeError> {
    let Some(this) = application.fields.iter().find(|f| f.name == "this") else {
        return Ok(None);
    };
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
        .map(Some)
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
pub(crate) async fn lift_rule<Env: QueryEnv>(
    application: &SyntaxApplication,
    scope: &Scope<'_>,
    env: &Env,
    analysis: &Working,
) -> Result<Effect, AnalyzeError> {
    let body = parse_rule_body(application)?;

    // ---- Head concept ----
    let head_descriptor = {
        let name = body.conclusion.as_str();
        let resolved = scope
            .resolve_concept(name, env)
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
        let proposition = lift_premise(premise, scope, env, analysis).await?;
        dialog_premises.push(DialogPremise::Assert(proposition));
    }
    for premise in body.unless {
        let proposition = lift_premise(premise, scope, env, analysis).await?;
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
async fn lift_premise<Env: QueryEnv>(
    premise: &NotationPremise,
    scope: &Scope<'_>,
    env: &Env,
    analysis: &Working,
) -> Result<Proposition, AnalyzeError> {
    let name = premise.concept.value.as_str();

    if let Some(formula) = lookup_formula(name) {
        return lift_formula_premise(premise, formula, scope, env, analysis).await;
    }

    let resolved = scope
        .resolve_concept(name, env)
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
            env,
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
                    env,
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
async fn lift_formula_premise<Env: QueryEnv>(
    premise: &NotationPremise,
    formula: FormulaInfo,
    scope: &Scope<'_>,
    env: &Env,
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
                    env,
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
    use dialog_artifacts::Entity;
    use dialog_query::AttributeDescriptor;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use dialog_query::the;
    use dialog_repository::Branch;
    use dialog_repository::helpers::{test_operator_with_profile, test_repo};
    use tonk_notation::parse;
    use tonk_schema::concept::AnonymousConcept;
    use tonk_schema::query_source::Source;

    /// Bound needed to query *and* commit through an operator —
    /// `QueryEnv` covers the resolution chain; `Provider<Publish>`
    /// covers the commit path the test fixtures use to assert
    /// concept facts onto the branch.
    trait FixtureEnv:
        tonk_schema::concept::QueryEnv + dialog_query::Provider<dialog_effects::memory::Publish>
    {
    }

    impl<T> FixtureEnv for T where
        T: tonk_schema::concept::QueryEnv + dialog_query::Provider<dialog_effects::memory::Publish>
    {
    }

    /// Branch + operator fixture used by the rule-side tests. The
    /// rule lifter walks the analyzer's resolution chain, which
    /// reads through `Source`; the fixture's [`declare`] helper
    /// asserts a concept on the branch with the right facts +
    /// published name so the analyzer can find it by name.
    struct Fixture<Op>
    where
        Op: FixtureEnv,
    {
        operator: Op,
        branch: Branch,
    }

    async fn new_fixture() -> Fixture<impl FixtureEnv> {
        let (operator, profile) = test_operator_with_profile().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo
            .branch("main")
            .open()
            .perform(&operator)
            .await
            .expect("test branch opens");
        Fixture { operator, branch }
    }

    impl<Op> Fixture<Op>
    where
        Op: FixtureEnv,
    {
        /// Assert one concept on the branch with the right
        /// attribute facts + an `id:<name>` referent claim so the
        /// analyzer's name lookup recovers the descriptor.
        async fn declare(&self, name: &str, descriptor: ConceptDescriptor) {
            let mut txn = self.branch.transaction();
            for (_, attr) in descriptor.with().iter() {
                let attr_entity: Entity = attr.to_uri().parse().expect("attribute URI");
                let type_label = attr
                    .content_type()
                    .and_then(|t| serde_json::to_value(t).ok())
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "String".to_owned());
                txn = txn
                    .assert(
                        the!("dialog.attribute/id")
                            .of(attr_entity.clone())
                            .is(format!("{}/{}", attr.domain(), attr.name())),
                    )
                    .assert(
                        the!("dialog.attribute/type")
                            .of(attr_entity.clone())
                            .is(type_label),
                    )
                    .assert(
                        the!("dialog.attribute/cardinality")
                            .of(attr_entity.clone())
                            .is("one".to_owned()),
                    )
                    .assert(
                        the!("dialog.meta/description")
                            .of(attr_entity)
                            .is(String::new()),
                    );
            }
            let concept_entity = descriptor.this();
            let id_entity: Entity = format!("id:{name}").parse().expect("id:<name> parses");
            txn = txn.assert(
                the!("dialog.name/referent")
                    .of(id_entity)
                    .is(concept_entity),
            );
            txn = txn.assert(AnonymousConcept::new(descriptor.clone()));
            txn.commit()
                .perform(&self.operator)
                .await
                .expect("concept assertion commits");
        }

        /// Run the analyzer against the fixture's branch.
        async fn analyze(
            &self,
            syntax: &tonk_notation::Syntax,
        ) -> Result<crate::analysis::Analysis<tonk_notation::Syntax>, AnalyzeError> {
            crate::analyzer::analyze(syntax, Source::from(&self.branch))
                .perform(&self.operator)
                .await
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
            tonk_core::transact::Statement::InstallEffect { effect, .. } => effect.clone(),
            other => panic!("expected Statement::InstallEffect, got {other:?}"),
        }
    }

    /// End-to-end happy path: a `rule!:` with one positive
    /// `when` premise lifts into a compiled `Effect` and lands
    /// in `analysis.mutate.statements` as a `Statement::InstallEffect`.
    #[dialog_common::test]
    async fn it_lifts_a_rule_into_an_effect() {
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;
        fixture
            .declare("pong", one_text_field("io.gozala.pong", "tag"))
            .await;

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

        let analysis = fixture
            .analyze(&syntax)
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
        let fixture = new_fixture().await;
        fixture
            .declare("ack", one_text_field("io.gozala.mailbox", "target"))
            .await;
        fixture
            .declare("message", one_text_field("io.gozala.mailbox", "body"))
            .await;

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
        let analysis = fixture
            .analyze(&syntax)
            .await
            .expect("analyze should succeed");

        assert_eq!(
            only_installed_effect(&analysis).polarity(),
            EffectPolarity::Retract
        );
    }

    /// `rule!:` with no `this:` lifts to an `Install` action whose
    /// override entity is `None` — the install lands at the
    /// content-derived `Effect::this`.
    #[dialog_common::test]
    async fn it_lifts_an_install_without_this_field() {
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;
        fixture
            .declare("pong", one_text_field("io.gozala.pong", "tag"))
            .await;

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let syntax = parse(doc).syntax.expect("parsed syntax");
        let analysis = fixture
            .analyze(&syntax)
            .await
            .expect("analyze should succeed");
        let statements = analysis.analysis.statements();
        assert_eq!(statements.len(), 1);
        let tonk_core::transact::Statement::InstallEffect { this, .. } = &statements[0].statement
        else {
            panic!("expected InstallEffect, got {:?}", statements[0].statement);
        };
        assert!(
            this.is_none(),
            "no this: in body, install entity stays None"
        );
    }

    /// `rule!: this: <uri>, assert!: ..` pins the install at a
    /// caller-chosen entity. The lifted `RuleAction::Install` carries
    /// `this: Some(<uri>)` and the analyzer pushes a
    /// `Statement::InstallEffect` whose `this` field matches.
    #[dialog_common::test]
    async fn it_lifts_an_install_at_chosen_entity() {
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;
        fixture
            .declare("pong", one_text_field("io.gozala.pong", "tag"))
            .await;

        let doc = "\
rule!:\n\
\x20 this: id:my-counter\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let syntax = parse(doc).syntax.expect("parsed syntax");
        let analysis = fixture
            .analyze(&syntax)
            .await
            .expect("analyze should succeed");
        let statements = analysis.analysis.statements();
        assert_eq!(statements.len(), 1);
        let tonk_core::transact::Statement::InstallEffect { this, .. } = &statements[0].statement
        else {
            panic!("expected InstallEffect, got {:?}", statements[0].statement);
        };
        let chosen: Entity = "id:my-counter".parse().expect("id URI parses");
        assert_eq!(
            this.as_ref(),
            Some(&chosen),
            "install-at entity should carry the user's `this: id:my-counter`"
        );
    }

    /// Unknown head concept name surfaces as
    /// [`AnalyzeErrorKind::UnknownConcept`].
    #[dialog_common::test]
    async fn it_rejects_unknown_head_concept() {
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;

        let doc = "\
rule!:\n\
\x20 assert!: missing-concept\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: {}\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
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
        let fixture = new_fixture().await;
        fixture
            .declare("pong", one_text_field("io.gozala.pong", "tag"))
            .await;

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: missing-premise\n\
\x20     where: {}\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
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
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;
        fixture
            .declare("pong", one_text_field("io.gozala.pong", "tag"))
            .await;

        let doc = "\
rule!:\n\
\x20 assert!: pong\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { wrong-field: ?tag }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
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
        let fixture = new_fixture().await;
        fixture
            .declare("counter", one_uint_field("io.gozala.counter", "count"))
            .await;
        fixture
            .declare("increment", one_uint_field("io.gozala.increment", "by"))
            .await;

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
        let analysis = fixture
            .analyze(&syntax)
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
        let fixture = new_fixture().await;
        fixture
            .declare("counter", one_uint_field("io.gozala.counter", "count"))
            .await;

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
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
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
        let fixture = new_fixture().await;
        fixture
            .declare("counter", one_uint_field("io.gozala.counter", "count"))
            .await;

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
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
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
