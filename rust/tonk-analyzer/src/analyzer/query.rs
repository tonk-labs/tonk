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
    collection_entry_terms, field_value_to_term, is_meta_field, untyped_descriptor,
    validate_claim_attribute,
};
use super::resolver_registry::{ResolverInfo, lookup_resolver};
use super::scope::Scope;
use crate::analyzer::Working;
use dialog_query::attribute::The;
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
    // `..: _` retracts; a query has nothing to retract. Skipping it
    // as a meta-key would leave a mistyped deletion (`head:` for
    // `head!:`) silently reading instead of failing.
    if let Some(rest) = query.fields.iter().find(|f| f.name == "..") {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::RestRetractionInQuery {
                head: query.predicate.source.clone(),
            },
            rest.name_range,
        ));
    }
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
            let this_term = this_term_for_query(&this);
            let mut terms = Parameters::new();
            let mut join: Vec<ConceptQuery> = Vec::new();
            terms.insert("this".into(), this_term.clone());
            for (field_name, attr) in descriptor.with().iter() {
                let user_field = query.fields.iter().find(|f| f.name == *field_name);
                // A keyed collection binds entries: the value under
                // the field, the key under the field's key operand.
                // A key the user left blank (or a field they omitted)
                // still surfaces, under an auto-named variable, the
                // same way `_` does for a value.
                if attr.the().attribute().is_none() {
                    let entries = match user_field {
                        Some(field) => collection_entry_terms(
                            field_name,
                            &field.value,
                            field.value_range,
                            scope,
                            analysis,
                            attr.content_type(),
                        )?,
                        None => vec![(
                            Term::<dialog_query::Any>::blank(),
                            Term::<dialog_query::Any>::var(field_name),
                        )],
                    };
                    // A `_` entry KEY or VALUE means "match anything
                    // and project it back", like every other query
                    // blank. (`collection_entry_terms` keeps blanks
                    // true blanks for the assert path, where
                    // `{key: _}` retracts.) Left anonymous it would
                    // match but surface nothing — the match row's
                    // field drops and a `show: {directory: _}` query
                    // reads as empty.
                    let mut entries = entries.into_iter().map(|(key, value)| {
                        let key = if key.is_blank() {
                            Term::<dialog_query::Any>::unique()
                        } else {
                            key
                        };
                        let value = if value.is_blank() {
                            Term::<dialog_query::Any>::unique()
                        } else {
                            value
                        };
                        (key, value)
                    });
                    let (key, value) = entries
                        .next()
                        .expect("collection_entry_terms yields at least one entry");
                    terms.insert(The::key_operand(field_name), key);
                    terms.insert(field_name.into(), value);
                    // A `Parameters` map holds one `(key, value)`
                    // slot pair per field, so every further entry
                    // rides a satellite query sharing the primary's
                    // `this` term; the evaluator equi-joins their
                    // frames, restoring "all entries on one match".
                    for (key, value) in entries {
                        let mut satellite = Parameters::new();
                        satellite.insert("this".into(), this_term.clone());
                        satellite.insert(The::key_operand(field_name), key);
                        satellite.insert(field_name.into(), value);
                        join.push(ConceptQuery {
                            terms: satellite,
                            predicate: descriptor.clone(),
                        });
                    }
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
                join,
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
                // Domain queries are OPEN-ENDED: a blank or variable
                // matches every value of the attribute regardless of
                // type, and a number literal matches the number it
                // names (`41`, `+41` and `41.0` all find the one stored
                // value, whatever wrote it). A declared attribute
                // lends only its CARDINALITY (a many-attribute must
                // enumerate every value, not select a winner) — never
                // a value type.
                let term = field_value_to_term(
                    &field.name,
                    &field.value,
                    field.value_range,
                    scope,
                    analysis,
                    None,
                )?;
                parameters.insert(field.name.clone(), term);
                if let Some(declared) = scope.attribute_by_id(&format!("{domain}/{}", field.name)) {
                    attributes.insert(field.name.clone(), untyped_descriptor(&declared.descriptor));
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

/// Parse explicit graph stages without treating their fields as root bindings.
pub(crate) fn graph_constraints(
    query: &SyntaxApplication,
    scope: &Scope,
    working: &Working,
) -> Result<Option<tonk_schema::graph::GraphConstraints>, AnalyzeError> {
    use tonk_notation::FieldValue;
    // A declared field always keeps its ordinary query semantics.
    if let HeadName::Concept(name) = &query.predicate.name {
        if scope.concept(name).is_some_and(|definition| {
            definition
                .descriptor
                .concept()
                .with()
                .iter()
                .any(|(name, _)| name == "graph")
        }) {
            return Ok(None);
        }
    }
    let Some(graph) = query.fields.iter().find(|field| field.name == "graph") else {
        return Ok(None);
    };
    let invalid = |reason: &str| {
        AnalyzeError::at(
            AnalyzeErrorKind::InvalidGraphQuery {
                reason: reason.into(),
            },
            graph.value_range,
        )
    };
    let FieldValue::Nested(stages) = &graph.value else {
        return Err(invalid("graph must contain matching and/or shaping maps"));
    };
    let mut constraints = tonk_schema::graph::GraphConstraints::default();
    for stage in stages {
        let target = match stage.name.as_str() {
            "matching" => &mut constraints.matching,
            "shaping" => &mut constraints.shaping,
            _ => return Err(invalid("graph stages are matching and shaping")),
        };
        let FieldValue::Nested(edges) = &stage.value else {
            return Err(invalid("each graph stage maps edges to field equalities"));
        };
        for edge in edges {
            let FieldValue::Nested(fields) = &edge.value else {
                return Err(invalid("each graph edge maps target fields to constants"));
            };
            let equalities = target.entry(edge.name.clone()).or_default();
            for field in fields {
                let value = field_value_to_term(
                    &field.name,
                    &field.value,
                    field.value_range,
                    scope,
                    working,
                    None,
                )?;
                let Term::Constant(value) = value else {
                    return Err(invalid("graph constraints must be constants"));
                };
                if equalities.insert(field.name.clone(), value).is_some() {
                    return Err(invalid("a graph equality may be stated only once"));
                }
            }
        }
    }
    Ok(Some(constraints))
}
