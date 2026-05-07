//! Query-side analysis — `build_query_application` plus its
//! `this`-slot helper. Queries can't carry an `&anchor` (the
//! parser rejects that), so the head intent here is just
//! [`ThisIntent`] with no name to publish.

use dialog_artifacts::Value;
use dialog_query::{Parameters, Term, concept::query::ConceptQuery};
use tonk_notation::{Field, HeadName, Query};

use super::assertion::derive_head_intent;
use super::error::AnalyzeError;
use super::field::{field_value_to_term, is_meta_field, user_field, validate_claim_attribute};
use super::resolver::Resolver;
use super::scope::Scope;
use crate::transact::{Analysis, Application, DomainApplication, ThisIntent};

pub(crate) async fn build_query_application<R: Resolver>(
    query: &Query,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Application, AnalyzeError> {
    // Queries can't carry an `&anchor` (parser rejects that), so
    // intent derivation only inspects `this:`. The returned name
    // is always `None` here.
    let (this, _name) = derive_head_intent(&query.fields, None, scope).await?;
    match &query.head.name {
        HeadName::Concept(concept_name) => {
            let resolved = scope
                .resolve_concept(concept_name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {concept_name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept {
                    name: concept_name.clone(),
                })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term_for_query(&this));
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                // Fields the user mentioned use whatever they
                // wrote (literal, variable, blank, etc.). Fields
                // they *omitted* default to a named variable so
                // matches surface the value in the response —
                // `person:` reads the same as
                // `person:\n  name: ?name\n  age: ?age`.
                let term = match user_field(query.fields.as_slice(), field_name) {
                    Some(value) => field_value_to_term(field_name, value, scope, analysis).await?,
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
                if resolved
                    .descriptor
                    .with()
                    .iter()
                    .all(|(n, _)| n != field.name)
                {
                    return Err(AnalyzeError::UnknownField {
                        concept: concept_name.clone(),
                        field: field.name.clone(),
                    });
                }
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
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
                return Err(AnalyzeError::ClaimWithoutFields {
                    domain: domain.clone(),
                });
            }
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term_for_query(&this));
            for field in &body_fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                },
                this,
                name: None,
            })
        }
        HeadName::Uri(uri) => Err(AnalyzeError::UnsupportedFieldValue {
            field: uri.clone(),
            form: "URI head in query (not yet implemented in Stage 2.1)",
        }),
    }
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
