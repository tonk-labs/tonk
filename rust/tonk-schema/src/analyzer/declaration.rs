//! Phase 1 helpers — parse `attribute!` and `concept!` bodies
//! into typed descriptors with content-derived entity URIs, and
//! build the `Application`s that the orchestrator caches by
//! source-expression index.

use dialog_artifacts::{Entity, Value};
use dialog_query::{
    AttributeDescriptor, ConceptDescriptor, Parameters, Term, concept::query::ConceptQuery,
};
use tonk_notation::{Assertion, FieldValue, Scalar};

use super::error::AnalyzeError;
use super::field::{is_meta_field, scalar_to_string};
use super::resolver::{ResolvedAttribute, Resolver};
use super::scope::Scope;
use crate::transact::{Application, ThisIntent};

/// Cached output of building an `attribute!` or `concept!` head
/// into its `Application`. Phase 1 builds these so the entity
/// URI is available early (for name registration in `scope`)
/// and Phase 3 emits the cached values without re-parsing the
/// body.
///
/// `inline_attributes` carries the anonymous attribute
/// definitions that appeared inside a `concept!`'s `with:` map.
/// Each is its own `Application` that Phase 3 emits *before*
/// the concept itself, so the attribute facts are queryable on
/// the branch by the time anything reads back. Empty for
/// `attribute!` heads.
pub(crate) struct DeclaredApplication {
    /// The head's own `Application`, ready to commit.
    pub application: Application,
    /// Anonymous attribute applications declared inline inside
    /// this concept's `with:` map. Empty for `attribute!` heads.
    pub inline_attributes: Vec<Application>,
}

/// Parsed `attribute!` body — descriptor plus entity URI.
pub(crate) struct AttributeBody {
    pub descriptor: AttributeDescriptor,
    pub entity: Entity,
}

pub(crate) fn parse_attribute_body(assertion: &Assertion) -> Result<AttributeBody, AnalyzeError> {
    parse_attribute_fields(&assertion.fields)
}

/// Parse an attribute definition's fields into a descriptor.
///
/// Used by `attribute! …:` heads and by inline
/// `with: { foo: { the: …, as: …, … } }` definitions nested
/// inside a `concept!` body. Same shape: `the`, `as`,
/// `cardinality`, and a *required* `description`.
pub(crate) fn parse_attribute_fields(
    fields: &[tonk_notation::Field],
) -> Result<AttributeBody, AnalyzeError> {
    let mut shape = serde_json::Map::new();
    for field in fields {
        // `this:` and `..:` are reserved meta-keys handled by the
        // outer assertion-binding flow; they don't contribute to
        // the attribute descriptor itself.
        if is_meta_field(&field.name) {
            continue;
        }
        let value_str = match &field.value {
            FieldValue::Literal(Scalar::String(s)) => s.clone(),
            FieldValue::Literal(other) => scalar_to_string(other)?,
            FieldValue::Uri(s) => s.clone(),
            FieldValue::Symbol(s) => {
                // Symbols in attribute-definition fields are
                // unusual (the `as:` and `cardinality:` slots
                // expect typed string literals like `text` /
                // `one`); the parser classified the lowercase
                // token as a Symbol, but for these slots we want
                // the literal text. Treat as the symbol's name.
                s.clone()
            }
            FieldValue::Variable(_) | FieldValue::Blank | FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedFieldValue {
                    field: field.name.clone(),
                    form: "non-literal (attribute definitions take literals)",
                });
            }
        };
        match field.name.as_str() {
            "the" | "as" | "cardinality" | "description" => {
                shape.insert(field.name.clone(), serde_json::Value::String(value_str));
            }
            other => {
                return Err(AnalyzeError::UnknownField {
                    concept: "attribute".into(),
                    field: other.into(),
                });
            }
        }
    }
    if !shape.contains_key("the") {
        return Err(AnalyzeError::InvalidAttributeBody {
            reason: "missing required field `the`".into(),
        });
    }
    let description_present = shape
        .get("description")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !description_present {
        return Err(AnalyzeError::InvalidAttributeBody {
            reason: "missing required field `description` (attribute \
                     definitions must include a non-empty description)"
                .into(),
        });
    }
    let descriptor: AttributeDescriptor = serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| AnalyzeError::InvalidAttributeBody {
            reason: e.to_string(),
        })?;
    let entity: Entity =
        descriptor
            .to_uri()
            .parse()
            .map_err(|e| AnalyzeError::InvalidAttributeBody {
                reason: format!("descriptor URI did not parse as entity: {e:?}"),
            })?;
    Ok(AttributeBody { descriptor, entity })
}

/// Parsed `concept!` body — descriptor plus entity URI plus any
/// inline attribute definitions that need to be registered as
/// their own meta-head plans alongside the concept's own.
pub(crate) struct ConceptBody {
    pub descriptor: ConceptDescriptor,
    pub entity: Entity,
    /// Attributes defined inline in the `with:` map (as opposed
    /// to referenced by name / URI). Each carries the descriptor
    /// needed to emit `dialog.attribute/{id,type,cardinality}`
    /// and `dialog.meta/description` claims so the attribute is
    /// queryable via `attribute:` after the `concept!` commits.
    pub inline_attributes: Vec<AttributeBody>,
}

pub(crate) async fn parse_concept_body<R: Resolver>(
    assertion: &Assertion,
    scope: &Scope<'_, R>,
) -> Result<ConceptBody, AnalyzeError> {
    let mut description: Option<String> = None;
    let mut with_fields: Vec<(String, ResolvedAttribute)> = Vec::new();
    let mut inline_attributes: Vec<AttributeBody> = Vec::new();
    for field in &assertion.fields {
        // `this:` and `..:` are reserved meta-keys handled by
        // the outer assertion-binding flow; they don't
        // contribute to the concept descriptor.
        if is_meta_field(&field.name) {
            continue;
        }
        match field.name.as_str() {
            "description" => {
                if let FieldValue::Literal(Scalar::String(s)) = &field.value {
                    description = Some(s.clone());
                } else {
                    return Err(AnalyzeError::UnsupportedFieldValue {
                        field: "description".into(),
                        form: "non-string literal",
                    });
                }
            }
            "with" => {
                let FieldValue::Nested(inner) = &field.value else {
                    return Err(AnalyzeError::InvalidConceptBody {
                        reason: "`with:` must be a mapping of field name → \
                                 attribute reference (bare symbol, `?var`, \
                                 URI) or inline attribute definition \
                                 (mapping with `the`/`as`/`cardinality`/\
                                 `description`)"
                            .into(),
                    });
                };
                for sub in inner {
                    if let FieldValue::Nested(attr_fields) = &sub.value {
                        // Inline attribute definition. Parse it
                        // as an attribute body and register it
                        // for emission as a separate meta-head
                        // plan.
                        let plan = parse_attribute_fields(attr_fields)?;
                        let resolved = ResolvedAttribute {
                            entity: plan.entity.clone(),
                            descriptor: plan.descriptor.clone(),
                        };
                        with_fields.push((sub.name.clone(), resolved));
                        inline_attributes.push(plan);
                    } else {
                        let resolved = resolve_concept_field(&sub.name, &sub.value, scope).await?;
                        with_fields.push((sub.name.clone(), resolved));
                    }
                }
            }
            other => {
                return Err(AnalyzeError::UnknownField {
                    concept: "concept".into(),
                    field: other.into(),
                });
            }
        }
    }
    if with_fields.is_empty() {
        return Err(AnalyzeError::InvalidConceptBody {
            reason: "`with:` is required and must declare at least one field".into(),
        });
    }
    let mut shape = serde_json::Map::new();
    if let Some(d) = &description {
        shape.insert("description".into(), serde_json::Value::String(d.clone()));
    }
    let with_obj: serde_json::Map<String, serde_json::Value> = with_fields
        .iter()
        .map(|(name, attr)| {
            (
                name.clone(),
                serde_json::to_value(&attr.descriptor)
                    .expect("AttributeDescriptor is serializable"),
            )
        })
        .collect();
    shape.insert("with".into(), serde_json::Value::Object(with_obj));
    let descriptor: ConceptDescriptor = serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| AnalyzeError::InvalidConceptBody {
            reason: e.to_string(),
        })?;
    let entity = descriptor.this();
    Ok(ConceptBody {
        descriptor,
        entity,
        inline_attributes,
    })
}

async fn resolve_concept_field<R: Resolver>(
    field_name: &str,
    value: &FieldValue,
    scope: &Scope<'_, R>,
) -> Result<ResolvedAttribute, AnalyzeError> {
    match value {
        FieldValue::Variable(name) => scope
            .resolve_attribute(name)
            .await
            .map_err(|e| AnalyzeError::ResolverFailed {
                context: format!("variable ?{name}"),
                reason: e.message,
            })?
            .ok_or_else(|| AnalyzeError::UnknownBookmark {
                field: field_name.into(),
                bookmark: name.clone(),
            }),
        FieldValue::Symbol(name) => scope
            .resolve_attribute(name)
            .await
            .map_err(|e| AnalyzeError::ResolverFailed {
                context: format!("symbol {name}"),
                reason: e.message,
            })?
            .ok_or_else(|| AnalyzeError::UnknownBookmark {
                field: field_name.into(),
                bookmark: name.clone(),
            }),
        FieldValue::Uri(uri) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeError::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            scope
                .resolve_attribute_by_entity(&entity)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("attribute entity {uri}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownBookmark {
                    field: field_name.into(),
                    bookmark: uri.clone(),
                })
        }
        _ => Err(AnalyzeError::UnsupportedFieldValue {
            field: field_name.into(),
            form: "expected a bare symbol (name lookup) or a URI \
                   (`xyz.tonk/foo`, `id:foo`, etc.)",
        }),
    }
}

/// Build the `Application` for an `attribute!` head — the
/// asserted predicate is the built-in `attribute` schema; the
/// `this` slot is the descriptor-derived entity URI; the
/// per-field terms carry the descriptor's id/type/cardinality/
/// description. The published name (`dialog.meta/name` claim on
/// `id:<name>`) is emitted by the planner from `Application`'s
/// `name` slot, not as a body parameter.
///
/// `AnonymousAttribute` requires all four claims to be present —
/// `ConceptByEntity` reconstruction depends on the full set —
/// so every field is emitted with an empty-string default for
/// `type` and `description` when the descriptor doesn't specify.
pub(crate) fn attribute_application(
    descriptor: &AttributeDescriptor,
    entity: &Entity,
    name: Option<String>,
) -> Application {
    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    terms.insert(
        "id".into(),
        Term::Constant(Value::String(format!(
            "{}/{}",
            descriptor.domain(),
            descriptor.name()
        ))),
    );
    let type_name = descriptor
        .content_type()
        .and_then(|ty| serde_json::to_value(ty).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    terms.insert("type".into(), Term::Constant(Value::String(type_name)));
    let cardinality_name = serde_json::to_value(descriptor.cardinality())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "one".into());
    terms.insert(
        "cardinality".into(),
        Term::Constant(Value::String(cardinality_name)),
    );
    terms.insert(
        "description".into(),
        Term::Constant(Value::String(descriptor.description().to_owned())),
    );
    Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: attribute_schema(),
        },
        this: ThisIntent::Uri(entity.clone()),
        name,
    }
}

/// Build the `Application` for a `concept!` head — the asserted
/// predicate is a synthesized concept-of-concept schema (one
/// `with.<field>` per field of the user's concept, plus the
/// `dialog.meta/concept` marker and `dialog.meta/description`).
/// The `this` slot is the descriptor-derived entity URI; the
/// published name is emitted by the planner from
/// `Application`'s `name` slot.
pub(crate) fn concept_application(
    descriptor: &ConceptDescriptor,
    entity: &Entity,
    name: Option<String>,
) -> Application {
    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    terms.insert(
        "concept".into(),
        Term::Constant(Value::Entity(
            "db:concept"
                .parse()
                .expect("`db:concept` is a valid entity URI"),
        )),
    );
    for (field_name, attr) in descriptor.with().iter() {
        let attr_entity: Entity = attr
            .to_uri()
            .parse()
            .expect("AttributeDescriptor::to_uri produces a valid entity");
        terms.insert(
            format!("with.{field_name}"),
            Term::Constant(Value::Entity(attr_entity)),
        );
    }
    if let Some(desc) = descriptor.description()
        && !desc.is_empty()
    {
        terms.insert(
            "description".into(),
            Term::Constant(Value::String(desc.to_owned())),
        );
    }
    Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: concept_schema(descriptor),
        },
        this: ThisIntent::Uri(entity.clone()),
        name,
    }
}

/// Build the `dialog.attribute` built-in schema descriptor. Its
/// fields map to the 5 EAVs every named attribute writes.
fn attribute_schema() -> ConceptDescriptor {
    fn cardinality_one() -> serde_json::Value {
        serde_json::Value::String("one".into())
    }
    let json = serde_json::json!({
        "with": {
            "id":          { "the": "dialog.attribute/id",          "as": "Text", "cardinality": cardinality_one() },
            "type":        { "the": "dialog.attribute/type",        "as": "Text", "cardinality": cardinality_one() },
            "cardinality": { "the": "dialog.attribute/cardinality", "as": "Text", "cardinality": cardinality_one() },
            "description": { "the": "dialog.meta/description",      "as": "Text", "cardinality": cardinality_one() },
            "name":        { "the": "dialog.meta/name",             "as": "Text", "cardinality": cardinality_one() },
        }
    });
    serde_json::from_value(json).expect("attribute schema is well-formed")
}

/// Build a `concept!` schema descriptor — one `with.<field>` per
/// field of the concept being defined, plus the
/// `dialog.meta/concept` marker (so branch-wide `concept:` queries
/// can find every concept entity) and optional name and
/// description fields.
fn concept_schema(descriptor: &ConceptDescriptor) -> ConceptDescriptor {
    let mut with = serde_json::Map::new();
    for (name, _attr) in descriptor.with().iter() {
        with.insert(
            format!("with.{name}"),
            serde_json::json!({
                "the": format!("dialog.concept.with/{name}"),
                "as": "Entity",
                "cardinality": "one",
            }),
        );
    }
    with.insert(
        "concept".into(),
        serde_json::json!({
            "the": "dialog.meta/concept",
            "as": "Entity",
            "cardinality": "one",
        }),
    );
    with.insert(
        "name".into(),
        serde_json::json!({
            "the": "dialog.meta/name",
            "as": "Text",
            "cardinality": "one",
        }),
    );
    with.insert(
        "description".into(),
        serde_json::json!({
            "the": "dialog.meta/description",
            "as": "Text",
            "cardinality": "one",
        }),
    );
    serde_json::from_value(serde_json::json!({ "with": with }))
        .expect("concept schema is well-formed")
}
