//! Phase 1 helpers — parse `attribute!` and `concept!` bodies
//! into typed descriptors with content-derived entity URIs, and
//! build the `Application`s that the orchestrator caches by
//! source-expression index.

use dialog_artifacts::{Entity, Value};
use dialog_query::{
    AttributeDescriptor, ConceptDescriptor, Parameters, Term, concept::query::ConceptQuery,
};
use tonk_notation::{Application as SyntaxApplication, FieldValue, Scalar};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::{is_meta_field, scalar_to_string};
use super::scope::Scope;
use tonk_schema::resolution::AttributeDefinition;
use tonk_schema::transact::{Application, ThisIntent};

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

pub(crate) fn parse_attribute_body(
    assertion: &SyntaxApplication,
) -> Result<AttributeBody, AnalyzeError> {
    parse_attribute_fields(&assertion.fields)
}

/// Parse an attribute definition's fields into a descriptor.
///
/// Used by `attribute!:` heads and by inline
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
        // Per-field value-shape requirements:
        //
        // - `as` accepts a Symbol (`text`), a string literal
        //   (`"Text"`) or a URI-like form. Translates to
        //   dialog's serde discriminant.
        // - `cardinality` is the same.
        // - `the` accepts a URI (`xyz.tonk/foo`) or a literal
        //   string holding the same shape.
        // - `description` requires a quoted string literal —
        //   bare symbols are rejected to discourage one-word
        //   non-descriptions like `description: recipe`.
        match field.name.as_str() {
            "as" => {
                let value_str = stringify_simple_value(field)?;
                let normalized = normalize_type_name(&value_str).ok_or_else(|| {
                    AnalyzeErrorKind::InvalidAttributeBody {
                        reason: format!(
                            "unknown attribute type {value_str:?} — \
                             expected one of: text, unsigned-integer, \
                             signed-integer, float, boolean, entity, \
                             bytes"
                        ),
                    }
                })?;
                shape.insert("as".into(), serde_json::Value::String(normalized.into()));
            }
            "cardinality" => {
                let value_str = stringify_simple_value(field)?;
                let normalized = normalize_cardinality_name(&value_str).ok_or_else(|| {
                    AnalyzeErrorKind::InvalidAttributeBody {
                        reason: format!(
                            "unknown cardinality {value_str:?} — \
                             expected `one` or `many`"
                        ),
                    }
                })?;
                shape.insert(
                    "cardinality".into(),
                    serde_json::Value::String(normalized.into()),
                );
            }
            "the" => {
                let value_str = stringify_simple_value(field)?;
                shape.insert("the".into(), serde_json::Value::String(value_str));
            }
            "description" => {
                let value_str = require_string_description(field)?;
                shape.insert("description".into(), serde_json::Value::String(value_str));
            }
            other => {
                return Err(AnalyzeErrorKind::UnknownField {
                    concept: "attribute".into(),
                    field: other.into(),
                }
                .into());
            }
        }
    }
    if !shape.contains_key("the") {
        return Err(AnalyzeErrorKind::InvalidAttributeBody {
            reason: "missing required field `the`".into(),
        }
        .into());
    }
    let description_present = shape
        .get("description")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !description_present {
        return Err(AnalyzeErrorKind::InvalidAttributeBody {
            reason: "missing required field `description` (attribute \
                     definitions must include a non-empty description)"
                .into(),
        }
        .into());
    }
    let descriptor: AttributeDescriptor = serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| AnalyzeErrorKind::InvalidAttributeBody {
            reason: e.to_string(),
        })?;
    let entity: Entity =
        descriptor
            .to_uri()
            .parse()
            .map_err(|e| AnalyzeErrorKind::InvalidAttributeBody {
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
    /// `true` when the body carried the `transient:` tag (bare
    /// key with no value, or the explicit `transient: true`).
    /// Drives emission of the `dialog.concept/transient` marker
    /// fact in [`concept_application`] so the reactor's effects
    /// loop classifies this concept's facts as transient.
    pub transient: bool,
    /// Attributes defined inline in the `with:` map (as opposed
    /// to referenced by name / URI). Each carries the descriptor
    /// needed to emit `dialog.attribute/{id,type,cardinality}`
    /// and `dialog.meta/description` claims so the attribute is
    /// queryable via `attribute:` after the `concept!` commits.
    pub inline_attributes: Vec<AttributeBody>,
}

pub(crate) fn parse_concept_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<ConceptBody, AnalyzeError> {
    let mut description: Option<String> = None;
    let mut transient: bool = false;
    let mut with_fields: Vec<(String, AttributeDefinition)> = Vec::new();
    let mut inline_attributes: Vec<AttributeBody> = Vec::new();
    // A concept's entity is content-derived from its descriptor by
    // default, but a `this: <uri>` pins it to a stable, chosen
    // entity (e.g. `tonk:view`) so the concept is referenceable by
    // that URI even if its published name later moves. Mirrors how
    // built-in concepts pin themselves to `db:<name>`.
    let mut pinned_entity: Option<Entity> = None;
    for field in &assertion.fields {
        // `..:` is the rest-retraction marker and never contributes
        // to a concept descriptor. `this:` is read below as the
        // optional entity pin; it likewise doesn't become a field.
        if field.name == ".." {
            continue;
        }
        if field.name == "this" {
            pinned_entity = parse_concept_this(field)?;
            continue;
        }
        match field.name.as_str() {
            "description" => {
                description = Some(require_string_description(field)?);
            }
            "transient" => {
                transient = parse_transient_tag(field)?;
            }
            "with" => {
                let FieldValue::Nested(inner) = &field.value else {
                    return Err(AnalyzeErrorKind::InvalidConceptBody {
                        reason: "`with:` must be a mapping of field name → \
                                 attribute reference (bare symbol, `?var`, \
                                 URI) or inline attribute definition \
                                 (mapping with `the`/`as`/`cardinality`/\
                                 `description`)"
                            .into(),
                    }
                    .into());
                };
                for sub in inner {
                    if let FieldValue::Nested(attr_fields) = &sub.value {
                        // Inline attribute definition. Parse it
                        // as an attribute body and register it
                        // for emission as a separate meta-head
                        // plan.
                        let plan = parse_attribute_fields(attr_fields)?;
                        let resolved = AttributeDefinition {
                            entity: plan.entity.clone(),
                            descriptor: plan.descriptor.clone(),
                        };
                        with_fields.push((sub.name.clone(), resolved));
                        inline_attributes.push(plan);
                    } else {
                        let resolved = resolve_concept_field(&sub.name, &sub.value, scope)?;
                        with_fields.push((sub.name.clone(), resolved));
                    }
                }
            }
            other => {
                return Err(AnalyzeErrorKind::UnknownField {
                    concept: "concept".into(),
                    field: other.into(),
                }
                .into());
            }
        }
    }
    if with_fields.is_empty() {
        return Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: "`with:` is required and must declare at least one field".into(),
        }
        .into());
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
        .map_err(|e| AnalyzeErrorKind::InvalidConceptBody {
            reason: e.to_string(),
        })?;
    // `this:` pins the entity; otherwise derive it from the
    // descriptor (content-addressed).
    let entity = pinned_entity.unwrap_or_else(|| descriptor.this());
    Ok(ConceptBody {
        descriptor,
        entity,
        transient,
        inline_attributes,
    })
}

/// Read a `concept!`'s `this:` field as an optional entity pin.
///
/// A URI (`tonk:view`, `did:key:…`, `id:…`, `db:…`) pins the
/// concept to that stable entity. Every other form — a `?var`
/// binding, a bare symbol, omitted — yields `None`: the entity is
/// content-derived from the descriptor and the `?var`/name intent
/// is handled by the head-intent flow (`derive_head_intent`), not
/// here.
fn parse_concept_this(field: &tonk_notation::Field) -> Result<Option<Entity>, AnalyzeError> {
    match &field.value {
        FieldValue::Uri(uri) => {
            let entity = uri
                .parse()
                .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                    AnalyzeError::at(
                        AnalyzeErrorKind::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        },
                        field.value_range,
                    )
                })?;
            Ok(Some(entity))
        }
        _ => Ok(None),
    }
}

/// Resolve one `with:`-map field reference to its
/// [`AttributeDefinition`] from the [`Scope`] tables. The resolve
/// phase has already populated those tables (in-doc declarations
/// plus any branch attributes the graph batched), so this is a
/// synchronous table read — a miss is an unknown-bookmark error.
fn resolve_concept_field(
    field_name: &str,
    value: &FieldValue,
    scope: &Scope,
) -> Result<AttributeDefinition, AnalyzeError> {
    match value {
        FieldValue::Variable(name) | FieldValue::Symbol(name) => {
            scope.attribute(name).ok_or_else(|| {
                AnalyzeErrorKind::UnknownNameReference {
                    field: field_name.into(),
                    name: name.clone(),
                }
                .into()
            })
        }
        FieldValue::Uri(uri) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeErrorKind::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            scope.attribute_by_entity(&entity).ok_or_else(|| {
                AnalyzeErrorKind::UnknownNameReference {
                    field: field_name.into(),
                    name: uri.clone(),
                }
                .into()
            })
        }
        _ => Err(AnalyzeErrorKind::UnsupportedFieldValue {
            field: field_name.into(),
            form: "expected a bare symbol (name lookup) or a URI \
                   (`xyz.tonk/foo`, `id:foo`, etc.)",
        }
        .into()),
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
    transient: bool,
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
    // `transient: true` adds a `(this, dialog.concept/transient,
    // db:transient)` marker fact. The synthesized
    // `concept_schema` includes a matching field; durable
    // concepts skip the term so no claim is emitted (the
    // emitter ignores fields whose term is absent).
    if transient {
        terms.insert(
            "transient".into(),
            Term::Constant(Value::Entity(
                "db:transient"
                    .parse()
                    .expect("`db:transient` is a valid entity URI"),
            )),
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
    with.insert(
        "transient".into(),
        serde_json::json!({
            "the": "dialog.concept/transient",
            "as": "Entity",
            "cardinality": "one",
        }),
    );
    serde_json::from_value(serde_json::json!({ "with": with }))
        .expect("concept schema is well-formed")
}

/// Translate a user-facing attribute type name into dialog's
/// serde discriminant.
///
/// The guide uses kebab-case-lowercase (`text`,
/// `unsigned-integer`, `signed-integer`, `float`, `boolean`,
/// `entity`, `bytes`); dialog's `Type` enum is PascalCase
/// (`Text`, `UnsignedInteger`, …). The analyzer translates at
/// the boundary so the user-facing surface is the only one
/// anyone has to remember.
///
/// PascalCase is also accepted so internal callers and schemas
/// authored before the guide rewrite work without
/// double-translation.
fn normalize_type_name(name: &str) -> Option<&'static str> {
    match name {
        "text" | "Text" => Some("Text"),
        "unsigned-integer" | "UnsignedInteger" => Some("UnsignedInteger"),
        "signed-integer" | "SignedInteger" => Some("SignedInteger"),
        "float" | "Float" => Some("Float"),
        "boolean" | "Boolean" => Some("Boolean"),
        "entity" | "Entity" => Some("Entity"),
        "bytes" | "Bytes" => Some("Bytes"),
        _ => None,
    }
}

/// Validate a user-facing cardinality name. Dialog's serde
/// format uses lowercase (`one`, `many`); historically
/// PascalCase was accepted too — preserve that for back-compat
/// while normalizing to lowercase.
fn normalize_cardinality_name(name: &str) -> Option<&'static str> {
    match name {
        "one" | "One" => Some("one"),
        "many" | "Many" => Some("many"),
        _ => None,
    }
}

/// Coerce a "simple" attribute-body field into a string. Used
/// for `the:`, `as:`, `cardinality:` — slots whose value is a
/// short typed token (URI / type name / cardinality keyword).
/// Symbols, URIs, and literals all flow through; variables,
/// blanks, and nested mappings are rejected.
fn stringify_simple_value(field: &tonk_notation::Field) -> Result<String, AnalyzeError> {
    Ok(match &field.value {
        FieldValue::Literal(Scalar::String(s)) => s.clone(),
        FieldValue::Literal(other) => scalar_to_string(other)?,
        FieldValue::Uri(s) => s.clone(),
        FieldValue::Symbol(s) => s.clone(),
        FieldValue::Variable(_)
        | FieldValue::Blank
        | FieldValue::Nested(_)
        | FieldValue::Premises(_) => {
            return Err(AnalyzeErrorKind::UnsupportedFieldValue {
                field: field.name.clone(),
                form: "non-literal (attribute definitions take literals)",
            }
            .into());
        }
    })
}

/// Coerce a `description:` field into its string content. Only
/// quoted string literals are accepted — bare symbols would
/// match the symbol charset (lowercase, no spaces, no
/// punctuation) and are almost always one-word filler like
/// `description: recipe`. Forcing quotes pushes authors toward
/// writing prose descriptions ("A recipe with a title and
/// ingredients") instead of repeating the concept's name.
fn require_string_description(field: &tonk_notation::Field) -> Result<String, AnalyzeError> {
    match &field.value {
        FieldValue::Literal(Scalar::String(s)) => Ok(s.clone()),
        FieldValue::Symbol(s) => Err(AnalyzeErrorKind::InvalidAttributeBody {
            reason: format!(
                "`description:` value {s:?} looks like a bare symbol — write a \
                 quoted string explaining what the entity represents \
                 (`description: \"…\"`)"
            ),
        }
        .into()),
        _ => Err(AnalyzeErrorKind::InvalidAttributeBody {
            reason: "`description:` must be a string".into(),
        }
        .into()),
    }
}

/// Interpret a `transient:` field on a `concept!:` body as a
/// presence tag. The bare key (`transient:` with no value)
/// parses as a YAML null and tags the concept as transient.
/// `transient: true` is also accepted for the user who reaches
/// for the explicit form. `transient: false` is rejected —
/// omit the key for durable concepts (the default) so the
/// surface stays uniform: presence means transient, absence
/// means durable.
fn parse_transient_tag(field: &tonk_notation::Field) -> Result<bool, AnalyzeError> {
    match &field.value {
        // `transient:` (no value) parses as Null. Treat the
        // bare key as the tag.
        FieldValue::Literal(Scalar::Null) => Ok(true),
        FieldValue::Literal(Scalar::Boolean(true)) => Ok(true),
        FieldValue::Literal(Scalar::Boolean(false)) => Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: "`transient: false` isn't meaningful — omit the `transient:` \
                         field entirely to declare a durable concept (the default)"
                .into(),
        }
        .into()),
        _ => Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: "`transient:` is a tag — write `transient:` (bare key, no value) \
                     to mark the concept transient, or omit the key for a durable \
                     concept (the default)"
                .into(),
        }
        .into()),
    }
}
