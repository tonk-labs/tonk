//! Rule-side analysis — lifts a parsed
//! [`tonk_notation::Rule`] into a compiled
//! [`crate::effect::Effect`].
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

use dialog_query::concept::query::ConceptQuery;
use dialog_query::premise::Premise as DialogPremise;
use dialog_query::{InductiveRule, Negation, Parameters, Proposition, Term};
use tonk_notation::{Premise as NotationPremise, Rule, RulePolarity};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::field_value_to_term;
use super::resolver::Resolver;
use super::scope::Scope;
use crate::effect::{Effect, EffectPolarity};
use crate::transact::Analysis;

/// Lift a parsed [`Rule`] into an [`Effect`] ready to install.
///
/// Each premise's concept and the head concept are resolved
/// through `scope`; missing names surface as
/// [`AnalyzeErrorKind::UnknownConcept`]. Each premise's
/// `where:` field is translated through the existing
/// [`field_value_to_term`] helper, so `?var` / literal /
/// symbol / blank forms behave consistently with the rest of
/// the analyzer.
pub(crate) async fn lift_rule<R: Resolver>(
    rule: &Rule,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
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
                        reason: e.message,
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
        resolved.descriptor
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

/// Resolve one notation premise into a dialog
/// [`Proposition::Concept`].
async fn lift_premise<R: Resolver>(
    premise: &NotationPremise,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Proposition, AnalyzeError> {
    let name = premise.concept.value.as_str();
    let resolved = scope
        .resolve_concept(name)
        .await
        .map_err(|e| {
            AnalyzeError::at(
                AnalyzeErrorKind::ResolverFailed {
                    context: format!("rule premise concept {name:?}"),
                    reason: e.message,
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
    let descriptor = resolved.descriptor;

    // Build the term map for this premise's `where:` bindings.
    // Every operand of the head concept must be present (either
    // bound by the user or auto-filled with an anonymous
    // variable so the engine has a slot for it).
    let mut terms = Parameters::new();

    // `this` slot: explicit user binding wins; otherwise mint a
    // unique anonymous variable so the premise can still match.
    let user_this = premise.bindings.iter().find(|f| f.name == "this");
    if let Some(field) = user_this {
        let term =
            field_value_to_term("this", &field.value, field.value_range, scope, analysis).await?;
        terms.insert("this".into(), term);
    } else {
        terms.insert("this".into(), Term::<dialog_query::Any>::unique());
    }

    // Per-field bindings declared by the user. Fields not
    // mentioned default to anonymous variables (consistent with
    // how `Query` bodies project unmentioned fields).
    for (field_name, _attr) in descriptor.with().iter() {
        if field_name == "this" {
            continue;
        }
        let user_binding = premise.bindings.iter().find(|f| f.name == *field_name);
        let term = match user_binding {
            Some(field) => {
                field_value_to_term(field_name, &field.value, field.value_range, scope, analysis)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::resolver::{ResolvedAttribute, ResolvedConcept, ResolverError};
    use async_trait::async_trait;
    use dialog_artifacts::Entity;
    use dialog_query::AttributeDescriptor;
    use dialog_query::artifact::Type;
    use dialog_query::attribute::Cardinality as DialogCardinality;
    use dialog_query::concept::descriptor::ConceptDescriptor;
    use std::collections::HashMap;
    use tonk_notation::parse;

    /// Resolver backed by an in-memory map of concept name →
    /// descriptor. Lets the test feed pre-built concepts to the
    /// lifter without standing up a branch.
    struct TestResolver {
        concepts: HashMap<String, ResolvedConcept>,
    }

    impl TestResolver {
        fn new() -> Self {
            Self {
                concepts: HashMap::new(),
            }
        }

        fn declare(&mut self, name: &str, descriptor: ConceptDescriptor) {
            let entity = descriptor.this();
            self.concepts
                .insert(name.to_string(), ResolvedConcept { entity, descriptor });
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl crate::analyzer::resolver::Resolver for TestResolver {
        async fn resolve_concept(
            &self,
            name: &str,
        ) -> Result<Option<ResolvedConcept>, ResolverError> {
            Ok(self.concepts.get(name).cloned())
        }
        async fn resolve_attribute(
            &self,
            _name: &str,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(None)
        }
        async fn resolve_attribute_by_entity(
            &self,
            _entity: &Entity,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(None)
        }
        async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolverError> {
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

    /// End-to-end happy path: a `rule!:` with one positive
    /// `when` premise lifts into a compiled `Effect` and lands
    /// in `analysis.effects`.
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

        assert_eq!(analysis.effects.len(), 1);
        let effect = &analysis.effects[0];
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

        assert_eq!(analysis.effects.len(), 1);
        assert_eq!(analysis.effects[0].polarity(), EffectPolarity::Retract);
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
}
