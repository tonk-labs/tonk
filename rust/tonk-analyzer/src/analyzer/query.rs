//! Query-side analysis — `build_query_application` plus its
//! `this`-slot helper. Queries can't carry an `&anchor` (the
//! parser rejects that), so the head intent here is just
//! [`ThisIntent`] with no name to publish.

use dialog_artifacts::Value;
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use tonk_notation::{Application as SyntaxApplication, Field, HeadName};

use super::assertion::derive_head_intent;
use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::{
    collection_entry_terms, field_value_to_term, is_meta_field, validate_claim_attribute,
};
use super::resolver_registry::{ResolverInfo, lookup_resolver};
use super::scope::Scope;
use crate::analyzer::Working;
use dialog_query::attribute::Relation;
use tonk_schema::transact::{Application, DomainApplication, ThisIntent};

pub(crate) fn build_query_application(
    query: &SyntaxApplication,
    scope: &Scope,
    analysis: &Working,
) -> Result<Application, AnalyzeError> {
    // Queries can't carry an `&anchor` (parser rejects that), so
    // intent derivation only inspects `this:`. The returned name
    // is always `None` here.
    let (this, _name) = derive_head_intent(&query.fields, None, scope)?;
    let head_range = query.predicate.range;
    match &query.predicate.name {
        HeadName::Concept(concept_name) => {
            // A `tree/*` resolver reads the store's own structure, so it
            // heads a query like a concept does — but it resolves from
            // dialog's resolver registry rather than the branch.
            if let Some(resolver) = lookup_resolver(concept_name) {
                return build_resolver_query(query, resolver, scope, analysis);
            }
            let resolved = scope.concept(concept_name).ok_or_else(|| {
                AnalyzeError::at(
                    AnalyzeErrorKind::UnknownConcept {
                        name: concept_name.clone(),
                    },
                    head_range,
                )
            })?;
            // Queries don't carry durability — unwrap the plain
            // dialog descriptor from the durability-tagged
            // [`ConceptDefinition`].
            let descriptor = resolved.descriptor.concept().clone();
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term_for_query(&this));
            for (field_name, attr) in descriptor.with().iter() {
                let user_field = query.fields.iter().find(|f| f.name == *field_name);
                // A keyed collection binds an entry: the value under
                // the field, the key under the field's key operand.
                // A key the user left blank (or a field they omitted)
                // still surfaces, under an auto-named variable, the
                // same way `_` does for a value.
                if attr.the().attribute().is_none() {
                    let (key, value) = match user_field {
                        Some(field) => collection_entry_terms(
                            field_name,
                            &field.value,
                            field.value_range,
                            scope,
                            analysis,
                            attr.content_type(),
                        )?,
                        None => (
                            Term::<dialog_query::Any>::blank(),
                            Term::<dialog_query::Any>::var(field_name),
                        ),
                    };
                    let key = if key.is_blank() {
                        Term::<dialog_query::Any>::unique()
                    } else {
                        key
                    };
                    terms.insert(Relation::key_operand(field_name), key);
                    terms.insert(field_name.into(), value);
                    continue;
                }
                // Fields the user mentioned use whatever they
                // wrote (literal, variable, blank, etc.). Fields
                // they *omitted* default to a named variable so
                // matches surface the value in the response —
                // `person:` reads the same as
                // `person:\n  name: ?name\n  age: ?age`.
                let term = match user_field {
                    Some(field) => field_value_to_term(
                        field_name,
                        &field.value,
                        field.value_range,
                        scope,
                        analysis,
                        attr.content_type(),
                    )?,
                    None => Term::<dialog_query::Any>::var(field_name),
                };
                terms.insert(field_name.into(), term);
            }
            // Reject unknown user-supplied fields. `this:` and
            // `..:` are reserved meta-keys (selecting the entity
            // and rest-of-attributes retraction respectively),
            // not real fields — exempt them.
            for field in &query.fields {
                if is_meta_field(&field.name) {
                    continue;
                }
                if descriptor.with().iter().all(|(n, _)| n != field.name) {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::UnknownField {
                            concept: concept_name.clone(),
                            field: field.name.clone(),
                        },
                        field.name_range,
                    ));
                }
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: descriptor,
                },
                this,
                name: None,
            })
        }
        HeadName::Claim(domain) => {
            // Filter `this:` and `..:` out before the
            // claim-needs-fields check so an expression whose
            // only body field is `this:` (no claim attributes)
            // surfaces the right "claim without fields" error.
            let body_fields: Vec<&Field> = query
                .fields
                .iter()
                .filter(|f| !is_meta_field(&f.name))
                .collect();
            if body_fields.is_empty() {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::ClaimWithoutFields {
                        domain: domain.clone(),
                    },
                    head_range,
                ));
            }
            let mut parameters = Parameters::new();
            let mut attributes = std::collections::BTreeMap::new();
            parameters.insert("this".into(), this_term_for_query(&this));
            for field in &body_fields {
                validate_claim_attribute(domain, &field.name, field.name_range)?;
                // Declared attributes govern the read side too: a
                // cardinality-many field must enumerate every value,
                // not select a winner — and a literal constant must
                // be typed by the declared attribute, or a bare `41`
                // (inferred signed) could never match the unsigned
                // value a declared write stored.
                let declared = scope.attribute_by_id(&format!("{domain}/{}", field.name));
                let expected = declared
                    .as_ref()
                    .and_then(|declared| declared.descriptor.content_type());
                let term = field_value_to_term(
                    &field.name,
                    &field.value,
                    field.value_range,
                    scope,
                    analysis,
                    expected,
                )?;
                parameters.insert(field.name.clone(), term);
                if let Some(declared) = declared {
                    attributes.insert(field.name.clone(), declared.descriptor);
                }
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                    attributes,
                },
                this,
                name: None,
            })
        }
        HeadName::Uri(uri) => Err(AnalyzeError::at(
            AnalyzeErrorKind::UnsupportedFieldValue {
                field: uri.clone(),
                form: "URI head in query (not yet implemented in Stage 2.1)",
            },
            head_range,
        )),
    }
}

/// Build a resolver query head (`tree/node:`, `tree/entry:`, …).
///
/// The body's fields are the resolver's operands, exactly as in premise
/// position: unknown ones are rejected against dialog's own schema, and
/// the ones a document omits stay unbound — a resolver's outputs are
/// values it produces, not requirements. Only its required input must
/// be bound, which dialog's planner enforces.
fn build_resolver_query(
    query: &SyntaxApplication,
    resolver: &ResolverInfo,
    scope: &Scope,
    analysis: &Working,
) -> Result<Application, AnalyzeError> {
    let mut terms = Parameters::new();
    for field in &query.fields {
        if is_meta_field(&field.name) {
            continue;
        }
        if !resolver.operands().any(|operand| operand == field.name) {
            let mut valid: Vec<&str> = resolver.operands().collect();
            valid.sort_unstable();
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::UnknownFormulaOperand {
                    formula: resolver.name.to_owned(),
                    operand: field.name.clone(),
                    valid: valid.join(", "),
                },
                field.name_range,
            ));
        }
        let term = field_value_to_term(
            &field.name,
            &field.value,
            field.value_range,
            scope,
            analysis,
            None,
        )?;
        terms.insert(field.name.clone(), term);
    }

    // Route through serde by name, the same way premises do, so the
    // analyzer never names resolver types by hand.
    let value = serde_json::json!({ "assert": resolver.name, "where": terms });
    let resolver_query: dialog_query::ResolverQuery =
        serde_json::from_value(value).map_err(|e| {
            AnalyzeError::at(
                AnalyzeErrorKind::UnknownConcept {
                    name: format!("{} ({e})", resolver.name),
                },
                query.predicate.range,
            )
        })?;

    Ok(Application::Resolver {
        query: Box::new(resolver_query),
        terms,
    })
}

fn this_term_for_query(this: &ThisIntent) -> Term<dialog_query::Any> {
    match this {
        ThisIntent::Variable(name) => Term::<dialog_query::Any>::var(name),
        ThisIntent::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
        ThisIntent::Derived => {
            // Dialog's engine requires `this` to be a named
            // variable (so it can bind matches to it) — a blank
            // surfaces as `UnboundVariable { variable_name: "this" }`
            // at evaluation. `Term::unique` mints `__N`, distinct
            // per call, so two anonymous queries don't end up
            // joining on a shared literal `"this"` name.
            Term::<dialog_query::Any>::unique()
        }
    }
}
