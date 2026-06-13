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
use dialog_query::constraint::Constraint;
use dialog_query::formula::query::FormulaQuery;
use dialog_query::premise::Premise as DialogPremise;
use dialog_query::{InductiveRule, Negation, Parameters, Proposition, Term};
use tonk_notation::{
    Application as SyntaxApplication, FieldValue, Premise as NotationPremise, Scalar,
};

use super::constraint::{ConstraintInfo, lookup_constraint};
use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::field_value_to_term;
use super::formula::{FormulaInfo, lookup_formula};
use super::scope::Scope;
use crate::analyzer::Working;
use dialog_artifacts::Entity;
use tonk_core::effect::{Effect, EffectPolarity};
use tonk_schema::rule::Rule;

/// Outcome of inspecting a `rule!:` claim body — install (build a
/// fresh [`Rule`]) or retract (resolve an existing rule from the
/// branch). Both paths produce a [`Rule`] value the analyzer hands
/// forward as `Application::Rule { rule, .. }`.
///
/// The two notation shapes:
///
/// - Install: `rule!: assert!: <head>, when: [...]` — body carries
///   polarity + premises, no `..: _`. A `this: <entity>` may
///   additionally pin the install at a user-chosen entity.
/// - Retract: `rule!: this: effect:<entity>, ..: _` — body carries
///   the rule's entity in `this:` and `..: _` as the
///   retract-everything sentinel.
pub(crate) enum RuleAction {
    /// Install a new rule. `rule` carries the freshly-built
    /// [`Rule`] with `Effect::source()` already captured; `this`
    /// carries `Some(entity)` when the user pinned the install with
    /// `this: <entity>`, `None` for the content-addressed default.
    Install {
        /// The freshly-built rule packaged for assert.
        rule: Box<Rule>,
        /// Caller-supplied install-at entity from `this: <entity>`.
        this: Option<Entity>,
    },
    /// Retract an installed rule. `rule` carries the [`Rule`]
    /// resolved off the branch with the stored `source` / `polarity`
    /// bytes — handed to `Statement::Retract` so the dissociate
    /// matches what was written byte-for-byte.
    Retract { rule: Box<Rule>, this: Entity },
}

/// `true` when the claim body is the rule-retract shape
/// (`this: <entity>` plus `..: _` and no `assert!:` / `retract!:`).
///
/// `..: _` is the syntactic marker that says "retract every
/// attribute of this entity"; combined with `this:` it identifies a
/// specific rule. Used by the analyzer to pick between
/// [`RuleAction::Install`] and [`RuleAction::Retract`].
pub(crate) fn is_rule_retract_body(application: &SyntaxApplication) -> bool {
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
///
/// For install the body is lifted to a fresh [`Effect`] then
/// packaged into a [`Rule`] via [`Rule::asserting`] /
/// [`Rule::asserting_at`]. For retract the [`Rule`] is resolved off
/// the branch via [`Rule::retracting`] so its carried `source` bytes
/// match what was stored — feeds straight into `Statement::Retract`
/// for a byte-exact dissociate.
pub(crate) fn lift_rule_claim(
    application: &SyntaxApplication,
    scope: &Scope,
    analysis: &Working,
) -> Result<Option<RuleAction>, AnalyzeError> {
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
        // The stored rule was read off the branch during resolve's
        // prefetch pass (so the carried `source` bytes match
        // what's installed) and cached on the scope. `Some(None)`
        // means the entity holds no `dialog.effect/source` claim —
        // the user is retracting a rule that isn't installed (or
        // already retracted in this document); propagate the
        // absent signal upward so the analyzer drops the claim
        // silently. `None` means resolve never prefetched it,
        // which is an analyzer bug.
        let resolved = scope.resolved_rule(&entity).ok_or_else(|| {
            AnalyzeError::at(
                AnalyzeErrorKind::RuleCompileFailed {
                    reason: format!("rule retract at {entity} was not prefetched during resolve"),
                },
                application.range,
            )
        })?;
        let Some(rule) = resolved else {
            return Ok(None);
        };
        Ok(Some(RuleAction::Retract {
            rule: Box::new(rule),
            this: entity,
        }))
    } else {
        let this = parse_rule_this_entity(application)?;
        let effect = lift_rule(application, scope, analysis)?;
        let rule = match this.clone() {
            Some(entity) => Rule::asserting_at(effect, entity),
            None => Rule::asserting(effect),
        };
        Ok(Some(RuleAction::Install {
            rule: Box::new(rule),
            this,
        }))
    }
}

/// Collect every concept a `rule!:` install body references — the
/// `assert!:` / `retract!:` head concept plus each `when:` /
/// `unless:` premise concept — as `(name, range)` pairs for the
/// graph's push phase to turn into concept [`Need`]s. Premise
/// concepts naming a built-in formula are skipped (they resolve
/// through the formula table, not the branch). Empty for retract
/// bodies, whose installed rule is resolved as a rule need instead.
///
/// [`Need`]: super::graph
pub(crate) fn collect_rule_concepts(
    application: &SyntaxApplication,
) -> Vec<(String, lsp_types::Range)> {
    if is_rule_retract_body(application) {
        return Vec::new();
    }
    let Ok(body) = parse_rule_body(application) else {
        // A malformed body surfaces its error later in `lift_rule`
        // with full diagnostics; collection just skips it.
        return Vec::new();
    };
    let mut out = vec![(body.conclusion.clone(), body.conclusion_range)];
    for premise in body.when.iter().chain(body.unless.iter()) {
        let name = premise.concept.value.as_str();
        if lookup_formula(name).is_some() {
            continue;
        }
        out.push((name.to_owned(), premise.concept.range));
    }
    out
}

/// Read an optional `this:` URI field out of a `rule!:` body.
///
/// Returns `Ok(None)` when no `this:` field is present. Returns
/// `Ok(Some(entity))` when the field carries a parseable URI.
/// Errors when the field is present but holds a non-URI value
/// (variable, blank, literal, …) — `rule!:` is a "named install"
/// or "named retract," not a templated one, so a `this: ?var` would
/// be ambiguous about which entity to write at.
pub(crate) fn parse_rule_this_entity(
    application: &SyntaxApplication,
) -> Result<Option<Entity>, AnalyzeError> {
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
pub(crate) fn lift_rule(
    application: &SyntaxApplication,
    scope: &Scope,
    analysis: &Working,
) -> Result<Effect, AnalyzeError> {
    let body = parse_rule_body(application)?;

    // ---- Head concept ----
    let head_descriptor = {
        let name = body.conclusion.as_str();
        let resolved = scope.concept(name).ok_or_else(|| {
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
        let proposition = lift_premise(premise, scope, analysis)?;
        dialog_premises.push(DialogPremise::Assert(proposition));
    }
    for premise in body.unless {
        let proposition = lift_premise(premise, scope, analysis)?;
        dialog_premises.push(DialogPremise::Unless(Negation(proposition)));
    }

    // ---- Reject trivially tautological assert rules ----
    // A rule is trivially tautological when its `assert!:` head
    // reads back what some positive `when:` premise already binds
    // with the same variable in every slot. The rule produces no
    // new information; the induce loop would spin against its
    // own output. Retract-polarity rules that read the head
    // concept are not tautological: they observe the fact and
    // remove it (the mailbox-with-ack pattern).
    if body.polarity == EffectPolarity::Assert
        && let Some(reason) = trivially_tautological(&head_descriptor, &dialog_premises)
    {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed { reason },
            application.range,
        ));
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

/// Return `Some(reason)` when the rule's head reads exactly back
/// what some positive `when:` premise binds — the rule has no
/// novel derivation. The check is intentionally narrow: it fires
/// only when the head's concept matches the premise's concept and
/// every head-side operand (every key in the head's `with:` map,
/// plus `this`) is bound to a variable that the premise also
/// binds under the same key with the same variable name.
///
/// Body premises that differ on the variable in any operand slot
/// (e.g. head reads `?count` while premise binds `?prev`) escape
/// this check — those rules legitimately derive new facts even if
/// they happen to share a head concept with a premise.
fn trivially_tautological(
    head: &dialog_query::ConceptDescriptor,
    premises: &[DialogPremise],
) -> Option<String> {
    use dialog_query::Term;
    let head_entity = head.this();
    for premise in premises {
        let DialogPremise::Assert(Proposition::Concept(query)) = premise else {
            continue;
        };
        if query.predicate.this() != head_entity {
            continue;
        }
        // The head's "operand set" is `this` plus every key the
        // head concept's `with:` map declares. For the rule to be
        // tautological, the premise's terms must bind each of
        // these to a variable with the same key name on both
        // sides (i.e. `?this` reads as `?this` and writes as
        // `?this`, not as a different variable).
        let head_keys = std::iter::once("this".to_owned())
            .chain(head.with().iter().map(|(name, _)| name.to_string()));
        let mut all_match = true;
        for key in head_keys {
            let term = query.terms.get(&key);
            let matches = matches!(
                term,
                Some(Term::Variable { name: Some(n), .. }) if n.as_str() == key.as_str()
            );
            if !matches {
                all_match = false;
                break;
            }
        }
        if all_match {
            return Some(format!(
                "rule is trivially tautological: the `assert!:` head reads back what the `when:` \
                 premise `{}` already binds. Either drop the rule, or change one of the premise's \
                 `where:` bindings so the head derives a different fact.",
                head.this(),
            ));
        }
    }
    None
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
/// A premise head names a built-in formula (`math/sum`,
/// `boolean/and`, …), a built-in constraint (`==`), or a concept.
/// Formulas and constraints are recognised first — they're fixed
/// sets that never live on the branch, so the registry lookups are
/// authoritative. Anything else resolves as a concept.
fn lift_premise(
    premise: &NotationPremise,
    scope: &Scope,
    analysis: &Working,
) -> Result<Proposition, AnalyzeError> {
    let name = premise.concept.value.as_str();

    if let Some(formula) = lookup_formula(name) {
        return lift_formula_premise(premise, formula, scope, analysis);
    }

    if let Some(constraint) = lookup_constraint(name) {
        return lift_constraint_premise(premise, constraint, scope, analysis);
    }

    let resolved = scope.concept(name).ok_or_else(|| {
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
        )?;
        terms.insert("this".into(), term);
    } else {
        terms.insert("this".into(), Term::<dialog_query::Any>::unique());
    }

    // Per-field bindings declared by the user. A field the user
    // omits, or writes as a blank (`_`), becomes a true
    // `Term::blank()` wildcard: a premise has no result block to
    // project a value into, so the query-side `_`-renders-back
    // substitution (which mints a named `__N` variable) does not
    // apply here. A named variable in a negation premise would be
    // treated as a required-but-unbound binding by the planner and
    // fail to compile; a blank is skipped as a wildcard.
    for (field_name, attr) in descriptor.with().iter() {
        if field_name == "this" {
            continue;
        }
        let user_binding = premise.bindings.iter().find(|f| f.name == *field_name);
        let term = match user_binding {
            Some(field) if matches!(field.value, FieldValue::Blank) => {
                Term::<dialog_query::Any>::blank()
            }
            Some(field) => field_value_to_term(
                field_name,
                &field.value,
                field.value_range,
                scope,
                analysis,
                attr.content_type(),
            )?,
            None => Term::<dialog_query::Any>::blank(),
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
fn lift_formula_premise(
    premise: &NotationPremise,
    formula: FormulaInfo,
    scope: &Scope,
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
            Some(field) => field_value_to_term(
                operand,
                &field.value,
                field.value_range,
                scope,
                analysis,
                None,
            )?,
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

/// Lift a premise whose head names a built-in constraint (`==`)
/// into a dialog [`Proposition::Constraint`].
///
/// A constraint relates the terms it's given — every operand is
/// required, with nothing to auto-fill (no `this` slot, no
/// `#[output]` slots). Validation mirrors [`lift_formula_premise`]:
///
/// - An operand the user wrote that the constraint doesn't have is
///   an [`AnalyzeErrorKind::UnknownFormulaOperand`].
/// - A required operand the user didn't write is a
///   [`AnalyzeErrorKind::MissingFormulaOperand`].
///
/// (The constraint reuses the formula operand diagnostics rather
/// than minting parallel variants — both are "this premise head's
/// fixed operand set doesn't match what you wrote.")
fn lift_constraint_premise(
    premise: &NotationPremise,
    constraint: &ConstraintInfo,
    scope: &Scope,
    analysis: &Working,
) -> Result<Proposition, AnalyzeError> {
    // Reject `where:` operands the constraint doesn't declare.
    for field in &premise.bindings {
        if !constraint.operands().any(|operand| operand == field.name) {
            let mut valid: Vec<&str> = constraint.operands().collect();
            valid.sort_unstable();
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnknownFormulaOperand {
                    formula: constraint.name.to_owned(),
                    operand: field.name.clone(),
                    valid: valid.join(", "),
                },
                field.name_range,
            ));
        }
    }

    // Translate every operand the constraint declares. All are
    // required — a constraint has nothing to compute, so an
    // unbound operand is an error rather than an anonymous fill.
    let mut terms = Parameters::new();
    for operand in constraint.operands() {
        let term = match premise.bindings.iter().find(|f| f.name == operand) {
            Some(field) => field_value_to_term(
                operand,
                &field.value,
                field.value_range,
                scope,
                analysis,
                None,
            )?,
            None => {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::MissingFormulaOperand {
                        formula: constraint.name.to_owned(),
                        operand: operand.to_owned(),
                    },
                    premise.concept.range,
                ));
            }
        };
        terms.insert(operand.to_string(), term);
    }

    // Construct the typed `Constraint` through its name-keyed serde
    // representation: `{"assert": <name>, "where": <terms>}`.
    // dialog-query owns the constraint↔name mapping, so routing
    // through serde keeps the analyzer from matching constraint
    // types by hand.
    let value = serde_json::json!({ "assert": constraint.name, "where": terms });
    let constraint_value: Constraint = serde_json::from_value(value).map_err(|e| {
        AnalyzeError::at(
            AnalyzeErrorKind::RuleCompileFailed {
                reason: format!("constraint {:?}: {e}", constraint.name),
            },
            premise.concept.range,
        )
    })?;

    Ok(Proposition::Constraint(constraint_value))
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
        tonk_schema::concept::QueryEnv
        + dialog_query::Provider<dialog_effects::memory::Publish>
        + dialog_query::Provider<dialog_effects::archive::Import>
    {
    }

    impl<T> FixtureEnv for T where
        T: tonk_schema::concept::QueryEnv
            + dialog_query::Provider<dialog_effects::memory::Publish>
            + dialog_query::Provider<dialog_effects::archive::Import>
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

    /// Build a `ConceptDescriptor` with one cardinality-one
    /// entity-reference field. Used to exercise digest behaviour
    /// for fields whose value names *another* entity.
    fn one_entity_field(domain: &str, name: &str) -> ConceptDescriptor {
        ConceptDescriptor::from(vec![(
            name,
            AttributeDescriptor::new(
                format!("{domain}/{name}").parse().unwrap(),
                "",
                DialogCardinality::One,
                Some(Type::Entity),
            ),
        )])
    }

    /// Extract the lone `Statement::InstallEffect` from an
    /// analysis tree, panicking if the document does not lower to
    /// exactly one statement and it is not an effect install.
    fn only_installed_effect(
        tree: &crate::analysis::Analysis<tonk_notation::Syntax>,
    ) -> tonk_core::effect::Effect {
        use tonk_schema::transact::{Application, Statement};
        let statements = tree.analysis.statements();
        assert_eq!(statements.len(), 1);
        match &statements[0].statement {
            Statement::Assert(Application::Rule { rule, .. }) => rule.effect.clone(),
            other => panic!("expected Statement::Assert(Application::Rule), got {other:?}"),
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
        use tonk_schema::transact::{Application, Statement, ThisIntent};
        let statements = analysis.analysis.statements();
        assert_eq!(statements.len(), 1);
        let Statement::Assert(Application::Rule { this, .. }) = &statements[0].statement else {
            panic!(
                "expected Statement::Assert(Application::Rule), got {:?}",
                statements[0].statement
            );
        };
        assert!(
            matches!(this, ThisIntent::Derived),
            "no this: in body, install entity is content-derived (Derived)"
        );
    }

    /// `rule!: this: <uri>, assert!: ..` pins the install at a
    /// caller-chosen entity. The lifted `RuleAction::Install` carries
    /// the chosen URI and the analyzer pushes a
    /// `Statement::Assert(Application::Rule { this: Uri(<uri>), .. })`.
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
        use tonk_schema::transact::{Application, Statement, ThisIntent};
        let statements = analysis.analysis.statements();
        assert_eq!(statements.len(), 1);
        let Statement::Assert(Application::Rule { this, .. }) = &statements[0].statement else {
            panic!(
                "expected Statement::Assert(Application::Rule), got {:?}",
                statements[0].statement
            );
        };
        let chosen: Entity = "id:my-counter".parse().expect("id URI parses");
        assert!(
            matches!(this, ThisIntent::Uri(e) if e == &chosen),
            "install-at entity should carry the user's `this: id:my-counter`, got {this:?}"
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

    /// A `rule!:` whose body reads the head concept back with the
    /// same variable in every slot is trivially tautological — it
    /// would re-emit a fact the body just observed. Reject it at
    /// compile time so the induce loop doesn't spin against its
    /// own output.
    #[dialog_common::test]
    async fn it_rejects_trivially_tautological_rule() {
        let fixture = new_fixture().await;
        fixture
            .declare("ping", one_text_field("io.gozala.ping", "tag"))
            .await;
        // Body's `where:` rebinds `this` and `tag` to variables
        // with the same names the head's `assert!:` reads — the
        // produced fact is identical to the read one.
        let doc = "\
rule!:\n\
\x20 assert!: ping\n\
\x20 when:\n\
\x20   - assert: ping\n\
\x20     where: { this: ?this, tag: ?tag }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::RuleCompileFailed { ref reason }
                    if reason.contains("trivially tautological")
            ),
            "expected RuleCompileFailed(trivially tautological), got {err:?}"
        );
    }

    /// A `rule!:` whose body reads the head concept but uses
    /// *different* variables in some slot still derives novel
    /// facts — don't reject it. The counter-increment shape
    /// (head writes `?count`, body reads `?prev`) is the canonical
    /// non-tautological case.
    #[dialog_common::test]
    async fn it_accepts_rule_with_distinct_variable_in_slot() {
        let fixture = new_fixture().await;
        fixture
            .declare("counter", one_uint_field("io.gozala.counter", "count"))
            .await;
        fixture
            .declare("increment", one_uint_field("io.gozala.increment", "by"))
            .await;
        // Head writes counter at `?count`; body reads counter at
        // `?prev`. Different variable → not tautological.
        let doc = "\
rule!:\n\
\x20 assert!: counter\n\
\x20 when:\n\
\x20   - assert: counter\n\
\x20     where: { this: ?this, count: ?prev }\n\
\x20   - assert: increment\n\
\x20     where: { this: ?this, by: 1 }\n\
\x20   - assert: math/sum\n\
\x20     where: { of: ?prev, with: 1, is: ?count }\n";
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        fixture
            .analyze(&syntax)
            .await
            .expect("counter-increment shape is not tautological");
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

    /// A `rule!:` whose body uses the `==` equality constraint to
    /// bind a fresh variable to a literal lifts into an effect —
    /// the empty-artifact shape (fill an unbound head field with a
    /// constant).
    #[dialog_common::test]
    async fn it_lifts_a_rule_with_an_equality_constraint_premise() {
        let fixture = new_fixture().await;
        fixture
            .declare("artifact", one_entity_field("io.gozala.artifact", "model"))
            .await;
        fixture
            .declare("request", one_text_field("io.gozala.request", "name"))
            .await;

        let doc = r#"
rule!:
  assert!: artifact
  when:
    - assert: request
      where: { this: ?this, name: ?name }
    - assert: ==
      where: { this: ?model, is: about:blank }
"#;
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

    /// A `where:` operand the `==` constraint doesn't have surfaces
    /// as [`AnalyzeErrorKind::UnknownFormulaOperand`] (constraints
    /// reuse the formula operand diagnostics).
    #[dialog_common::test]
    async fn it_rejects_unknown_constraint_operand() {
        let fixture = new_fixture().await;
        fixture
            .declare("artifact", one_entity_field("io.gozala.artifact", "model"))
            .await;

        let doc = r#"
rule!:
  assert!: artifact
  when:
    - assert: ==
      where: { this: ?model, equals: about:blank }
"#;
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownFormulaOperand { ref formula, ref operand, .. }
                    if formula == "==" && operand == "equals"
            ),
            "expected UnknownFormulaOperand(==, equals), got {err:?}"
        );
    }

    /// Omitting a required `==` operand surfaces as
    /// [`AnalyzeErrorKind::MissingFormulaOperand`] — a constraint
    /// relates the terms it's given, so a missing one is an error
    /// rather than an anonymous fill.
    #[dialog_common::test]
    async fn it_rejects_missing_constraint_operand() {
        let fixture = new_fixture().await;
        fixture
            .declare("artifact", one_entity_field("io.gozala.artifact", "model"))
            .await;

        let doc = r#"
rule!:
  assert!: artifact
  when:
    - assert: ==
      where: { this: ?model }
"#;
        let parsed = parse(doc);
        let syntax = parsed.syntax.expect("parsed syntax");
        let err = fixture.analyze(&syntax).await.expect_err("should fail");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::MissingFormulaOperand { ref formula, ref operand }
                    if formula == "==" && operand == "is"
            ),
            "expected MissingFormulaOperand(==, is), got {err:?}"
        );
    }

    // -- cross-path convergence ----------------------------------- //

    /// The notation analyzer's `Derived` lowering and the wire-path
    /// [`application_plan_from_predicate`] hash `(predicate, payload)`
    /// the same way, so a `view!:` written in YAML and a `view`
    /// command issued over `/transact` with matching attributes
    /// converge on the same subject entity.
    #[dialog_common::test]
    async fn it_converges_notation_and_wire_path_entities() {
        use dialog_artifacts::Value;
        use tonk_core::claim::{
            ConceptDescriptor as DurableConceptDescriptor, PredicateApplication, ValueMap,
        };
        use tonk_schema::transact::{
            Application, ApplicationPlan, Statement, application_plan_from_predicate,
        };

        let fixture = new_fixture().await;
        let descriptor = one_text_field("xyz.tonk.view", "name");
        fixture.declare("view", descriptor.clone()).await;

        // Notation path: a `view!:` with one literal field. No
        // `this:` and no `&anchor`, so the lowering falls into
        // `ThisIntent::Derived` and hashes `(predicate, body)`.
        let doc = "\
view!:\n\
\x20 name: Basic\n";
        let syntax = parse(doc).syntax.expect("parsed syntax");
        let analysis = fixture
            .analyze(&syntax)
            .await
            .expect("notation document analyzes");
        let notation_this = {
            let stmts = analysis.analysis.statements();
            assert_eq!(stmts.len(), 1, "expected one statement");
            let Statement::Assert(Application::Concept { query, .. }) = &stmts[0].statement else {
                panic!("expected concept assert, got {:?}", stmts[0].statement);
            };
            match query.terms.get("this").expect("this present") {
                dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
                other => panic!("expected entity constant, got {other:?}"),
            }
        };

        // Wire path: same descriptor, same payload, no `this:`.
        let mut parameters = ValueMap::new();
        parameters.insert("name".into(), Value::String("Basic".into()));
        let wire_plan = application_plan_from_predicate(PredicateApplication {
            predicate: DurableConceptDescriptor::Durable(descriptor),
            parameters,
            name: None,
        });
        let wire_this = {
            let ApplicationPlan::Concept(plan) = &wire_plan else {
                panic!("expected concept plan");
            };
            match plan.statement.terms.get("this").expect("this present") {
                dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
                other => panic!("expected entity constant, got {other:?}"),
            }
        };

        assert_eq!(
            notation_this, wire_this,
            "notation and wire paths must derive the same subject entity for the same (concept, payload)"
        );
    }

    /// Entity derivation includes reference fields, not just
    /// literals — so a concept identified by an entity *reference*
    /// (e.g. a view's `model`) gets a distinct, correct entity.
    ///
    /// Regression guard for a `body_digest` defect: it used to
    /// include only literal scalars and skip references, so two
    /// assertions that differed *only* by a reference field hashed
    /// to the **same** entity, and the notation path diverged from
    /// the wire path (`application_plan_from_predicate`), which
    /// always included the reference. Two facets are asserted:
    ///   1. distinctness — different `model` ⇒ different entity;
    ///   2. convergence  — notation and wire paths agree.
    #[dialog_common::test]
    async fn it_derives_distinct_entities_for_distinct_reference_fields() {
        use dialog_artifacts::Value;
        use tonk_core::claim::{
            ConceptDescriptor as DurableConceptDescriptor, PredicateApplication, ValueMap,
        };
        use tonk_schema::transact::{
            Application, ApplicationPlan, Statement, application_plan_from_predicate,
        };

        let fixture = new_fixture().await;

        // A `view` concept whose sole field, `model`, is an entity
        // reference, plus two distinct concepts it can point at.
        let view = one_entity_field("xyz.tonk.view", "model");
        fixture.declare("view", view.clone()).await;
        fixture
            .declare("counter", one_text_field("xyz.tonk.counter", "count"))
            .await;
        fixture
            .declare("greeting", one_text_field("xyz.tonk.greeting", "message"))
            .await;

        // Helper: lower a `view!: { model: <name> }` document and
        // pull the derived `this` entity out of the lone statement.
        async fn notation_this_for(fixture: &Fixture<impl FixtureEnv>, model_name: &str) -> Entity {
            let doc = format!("view!:\n  model: {model_name}\n");
            let syntax = parse(&doc).syntax.expect("parsed syntax");
            let analysis = fixture
                .analyze(&syntax)
                .await
                .expect("notation document analyzes");
            let stmts = analysis.analysis.statements();
            assert_eq!(stmts.len(), 1, "expected one statement");
            let Statement::Assert(Application::Concept { query, .. }) = &stmts[0].statement else {
                panic!("expected concept assert, got {:?}", stmts[0].statement);
            };
            match query.terms.get("this").expect("this present") {
                dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
                other => panic!("expected entity constant, got {other:?}"),
            }
        }

        let view_of_counter = notation_this_for(&fixture, "counter").await;
        let view_of_greeting = notation_this_for(&fixture, "greeting").await;

        // Facet 1 — distinctness. Today both drop `model` from the
        // digest and collide on `derive_this(view, {})`.
        assert_ne!(
            view_of_counter, view_of_greeting,
            "views for different models must be different entities; \
             body_digest dropping the `model` reference makes them collide"
        );

        // Facet 2 — convergence with the wire path for the counter
        // view. The notation analyzer resolves the bare symbol
        // `counter` to its concept entity via the published
        // `id:counter` name, which `declare` set to the descriptor's
        // own `this()`. The wire payload carries that same resolved
        // entity, so both paths see identical `(predicate, payload)`.
        let counter_concept = one_text_field("xyz.tonk.counter", "count").this();
        let mut parameters = ValueMap::new();
        parameters.insert("model".into(), Value::Entity(counter_concept));
        let wire_plan = application_plan_from_predicate(PredicateApplication {
            predicate: DurableConceptDescriptor::Durable(view),
            parameters,
            name: None,
        });
        let wire_this = {
            let ApplicationPlan::Concept(plan) = &wire_plan else {
                panic!("expected concept plan");
            };
            match plan.statement.terms.get("this").expect("this present") {
                dialog_query::Term::Constant(Value::Entity(e)) => e.clone(),
                other => panic!("expected entity constant, got {other:?}"),
            }
        };

        assert_eq!(
            view_of_counter, wire_this,
            "notation and wire paths must derive the same entity when the body \
             carries a `model` reference"
        );
    }

    /// A reference field that doesn't resolve is an error, not a
    /// silent skip. Dropping the unresolved `model` from the digest
    /// would fold the view into the entity it would have had with
    /// no model at all — deriving the wrong subject. The anchor
    /// (`&broken`) routes derivation through `body_digest` in the
    /// resolve pass, so this exercises that path specifically.
    #[dialog_common::test]
    async fn it_rejects_unresolved_reference_in_derived_entity() {
        let fixture = new_fixture().await;
        // Declare `view` (so the head concept resolves) but NOT the
        // concept its `model` points at.
        fixture
            .declare("view", one_entity_field("xyz.tonk.view", "model"))
            .await;

        let doc = "\
view!: &broken\n\
\x20 model: nonexistent\n";
        let syntax = parse(doc).syntax.expect("parsed syntax");
        let err = fixture
            .analyze(&syntax)
            .await
            .expect_err("unresolved reference in a derived entity must error");
        assert!(
            matches!(
                err.kind,
                AnalyzeErrorKind::UnknownNameReference { ref name, .. }
                    if name == "nonexistent"
            ),
            "expected UnknownNameReference(nonexistent), got {err:?}"
        );
    }
}
