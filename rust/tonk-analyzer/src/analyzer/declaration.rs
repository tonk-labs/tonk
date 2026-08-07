//! Phase 1 helpers — parse `attribute!` and `concept!` bodies
//! into typed descriptors with content-derived entity URIs, and
//! build the `Application`s that the orchestrator caches by
//! source-expression index.

use dialog_artifacts::{Entity, Value};
use dialog_query::{
    AttributeDescriptor, ConceptDescriptor, ConceptFieldDescriptor, Parameters, Term,
    concept::query::ConceptQuery,
};
use std::collections::BTreeMap;

use tonk_notation::{Application as SyntaxApplication, Field as SyntaxField, FieldValue, Scalar};

use super::error::{AnalyzeError, AnalyzeErrorKind};
use super::field::{is_meta_field, scalar_to_string, scalar_to_value};
use super::scope::Scope;
use tonk_core::command::CommandSchema;
use tonk_core::meta::AnchorName;
use tonk_schema::projection::{
    ControlProperty, ControlSource, EventAction, EventMember, ProjectionDescriptor,
    ProjectionSource, TargetMember,
};
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
    /// The head's own `Application`, ready to commit. `None` for a
    /// retraction-only `concept!:` body (`with: { f: _ }` / `..: _`
    /// with no asserted fields), which emits only `retractions`.
    pub application: Option<Application>,
    /// Anonymous attribute applications declared inline inside
    /// this concept's `with:` map. Empty for `attribute!` heads.
    pub inline_attributes: Vec<Application>,
    /// Per-field retraction applications emitted as
    /// `Statement::Retract` — one per `concept!:` field retraction
    /// (`with: { f: _ }` / `..: _`). Empty otherwise.
    pub retractions: Vec<Application>,
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

/// A field the concept body asks to retract via `field: _`,
/// regardless of which block (`with:`/`maybe:`) it appeared under.
/// The emitter resolves the field's stored attribute (and its
/// optional sibling) from the branch, so the block is immaterial
/// here — only the name matters.
pub(crate) struct RetractedField {
    pub name: String,
}

/// `..: _` rest-retraction: the body asks to drop *every* stored
/// field of the concept. The concrete field set isn't known from
/// the source — it's read from the concept's existing facts on the
/// branch at emission time.
pub(crate) struct RestRetraction;

/// Parsed `concept!` body — descriptor plus entity URI plus any
/// inline attribute definitions that need to be registered as
/// their own meta-head plans alongside the concept's own.
pub(crate) struct ConceptBody {
    pub descriptor: ConceptDescriptor,
    pub entity: Entity,
    /// Fields named for retraction via `field: _`. Never enter the
    /// descriptor's with-map (a retracted field must not become a
    /// required field, feed `concept_schema`, or shift the
    /// content-derived entity).
    pub retracted: Vec<RetractedField>,
    /// `Some` when the body carried `..: _` — retract all stored
    /// fields of the concept.
    pub rest_retraction: Option<RestRetraction>,
    /// `true` when the body asserts no fields (a retraction-only
    /// `concept!:`). `descriptor` then holds an unemitted stub; the
    /// graph skips the concept assertion and emits only retractions.
    pub asserts_nothing: bool,
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

/// Parsed nominal command body. Command identity is selected by the
/// declaration head; this carries only the typed argument schema and
/// inline attribute definitions that must be emitted first.
pub(crate) struct CommandBody {
    pub schema: CommandSchema,
}

/// Parsed event projection body before the declaration head selects
/// the projection entity.
pub(crate) struct ProjectionBody {
    pub descriptor: ProjectionDescriptor,
}

/// Parse and statically validate a `projection!:` body.
pub(crate) fn parse_projection_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<ProjectionBody, AnalyzeError> {
    let mut command = None;
    let mut default = false;
    let mut arguments = indexmap::IndexMap::new();
    let mut actions = Vec::new();
    for field in &assertion.fields {
        if field.name == "this" {
            continue;
        }
        match field.name.as_str() {
            "command" => {
                let resolved = match &field.value {
                    FieldValue::Symbol(name) | FieldValue::Literal(Scalar::String(name)) => {
                        scope.command(name)
                    }
                    FieldValue::Uri(uri) => uri
                        .parse::<Entity>()
                        .ok()
                        .and_then(|entity| scope.command_by_entity(&entity)),
                    _ => None,
                };
                command = Some(resolved.ok_or_else(|| {
                    AnalyzeError::at(
                        AnalyzeErrorKind::InvalidProjectionBody {
                            reason: "`command:` does not resolve to a nominal command".into(),
                        },
                        field.value_range,
                    )
                })?);
            }
            "default" => match &field.value {
                FieldValue::Literal(Scalar::Boolean(value)) => default = *value,
                _ => {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::InvalidProjectionBody {
                            reason: "`default:` must be a boolean".into(),
                        },
                        field.value_range,
                    ));
                }
            },
            "arguments" => {
                let FieldValue::Nested(fields) = &field.value else {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::InvalidProjectionBody {
                            reason: "`arguments:` must be a field-to-source mapping".into(),
                        },
                        field.value_range,
                    ));
                };
                for argument in fields {
                    arguments.insert(argument.name.clone(), parse_projection_source(argument)?);
                }
            }
            "actions" => {
                let FieldValue::List(items) = &field.value else {
                    return Err(AnalyzeError::at(
                        AnalyzeErrorKind::InvalidProjectionBody {
                            reason: "`actions:` must be a list".into(),
                        },
                        field.value_range,
                    ));
                };
                for item in items {
                    let action = match item.value.as_str() {
                        "prevent-default" => EventAction::PreventDefault,
                        "stop-propagation" => EventAction::StopPropagation,
                        "stop-immediate-propagation" => EventAction::StopImmediatePropagation,
                        other => {
                            return Err(AnalyzeError::at(
                                AnalyzeErrorKind::InvalidProjectionBody {
                                    reason: format!("unsupported event action {other:?}"),
                                },
                                item.range,
                            ));
                        }
                    };
                    actions.push(action);
                }
            }
            other => {
                return Err(AnalyzeError::at(
                    AnalyzeErrorKind::InvalidProjectionBody {
                        reason: format!("unknown projection field {other:?}"),
                    },
                    field.name_range,
                ));
            }
        }
    }
    let command = command.ok_or_else(|| {
        AnalyzeError::at(
            AnalyzeErrorKind::InvalidProjectionBody {
                reason: "missing required field `command`".into(),
            },
            assertion.range,
        )
    })?;
    for field in arguments.keys() {
        if !command.schema().required.contains_key(field)
            && !command.schema().optional.contains_key(field)
        {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidProjectionBody {
                    reason: format!("unknown command argument {field:?}"),
                },
                assertion
                    .fields
                    .iter()
                    .find(|candidate| candidate.name == "arguments")
                    .map(|candidate| candidate.value_range)
                    .unwrap_or(assertion.range),
            ));
        }
    }
    for required in command.schema().required.keys() {
        if !arguments.contains_key(required) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidProjectionBody {
                    reason: format!("required command argument {required:?} has no source"),
                },
                assertion.range,
            ));
        }
    }
    Ok(ProjectionBody {
        descriptor: ProjectionDescriptor {
            command: command.kind().clone(),
            default,
            arguments,
            actions,
        },
    })
}

fn parse_projection_source(field: &SyntaxField) -> Result<ProjectionSource, AnalyzeError> {
    let FieldValue::Nested(entries) = &field.value else {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::InvalidProjectionBody {
                reason: format!("source for argument {:?} must be a mapping", field.name),
            },
            field.value_range,
        ));
    };
    if entries.len() != 1 {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::InvalidProjectionBody {
                reason: format!(
                    "source for argument {:?} must contain exactly one source key",
                    field.name
                ),
            },
            field.value_range,
        ));
    }
    let source = &entries[0];
    let simple = || projection_source_name(source);
    match source.name.as_str() {
        "control" => match &source.value {
            FieldValue::Nested(options) => {
                let name = options
                    .iter()
                    .find(|option| option.name == "name")
                    .ok_or_else(|| invalid_projection("control source is missing `name`", source))
                    .and_then(projection_source_name)?;
                let property = match options.iter().find(|option| option.name == "property") {
                    None => ControlProperty::Value,
                    Some(option) => match projection_source_name(option)?.as_str() {
                        "value" => ControlProperty::Value,
                        "checked" => ControlProperty::Checked,
                        other => {
                            return Err(invalid_projection(
                                format!("unsupported control property {other:?}"),
                                option,
                            ));
                        }
                    },
                };
                if options
                    .iter()
                    .any(|option| option.name != "name" && option.name != "property")
                {
                    return Err(invalid_projection("unknown control source option", source));
                }
                Ok(ProjectionSource::Control(ControlSource { name, property }))
            }
            _ => Ok(ProjectionSource::Control(ControlSource {
                name: simple()?,
                property: ControlProperty::Value,
            })),
        },
        "data" => Ok(ProjectionSource::Data(simple()?)),
        "event" => Ok(ProjectionSource::Event(match simple()?.as_str() {
            "type" => EventMember::Type,
            "key" => EventMember::Key,
            "code" => EventMember::Code,
            "repeat" => EventMember::Repeat,
            "shiftKey" => EventMember::ShiftKey,
            "ctrlKey" => EventMember::CtrlKey,
            "altKey" => EventMember::AltKey,
            "metaKey" => EventMember::MetaKey,
            "button" => EventMember::Button,
            "clientX" => EventMember::ClientX,
            "clientY" => EventMember::ClientY,
            "timeStamp" => EventMember::TimeStamp,
            other => {
                return Err(invalid_projection(
                    format!("unsupported event member {other:?}"),
                    source,
                ));
            }
        })),
        "detail" => Ok(ProjectionSource::Detail(simple()?)),
        "target" => Ok(ProjectionSource::Target(match simple()?.as_str() {
            "value" => TargetMember::Value,
            "checked" => TargetMember::Checked,
            other => {
                return Err(invalid_projection(
                    format!("unsupported target member {other:?}"),
                    source,
                ));
            }
        })),
        "literal" => Ok(ProjectionSource::Literal(match &source.value {
            FieldValue::Literal(value) => scalar_to_value(value, None)?,
            FieldValue::Symbol(value) => Value::String(value.clone()),
            FieldValue::Uri(value) => Value::Entity(value.parse().map_err(|error| {
                invalid_projection(format!("invalid literal entity: {error}"), source)
            })?),
            _ => {
                return Err(invalid_projection(
                    "literal source must be a scalar",
                    source,
                ));
            }
        })),
        other => Err(invalid_projection(
            format!("unsupported projection source {other:?}"),
            source,
        )),
    }
}

fn projection_source_name(field: &SyntaxField) -> Result<String, AnalyzeError> {
    match &field.value {
        FieldValue::Symbol(value)
        | FieldValue::Uri(value)
        | FieldValue::Literal(Scalar::String(value)) => Ok(value.clone()),
        _ => Err(invalid_projection(
            "source option must be a scalar name",
            field,
        )),
    }
}

fn invalid_projection(reason: impl Into<String>, field: &SyntaxField) -> AnalyzeError {
    AnalyzeError::at(
        AnalyzeErrorKind::InvalidProjectionBody {
            reason: reason.into(),
        },
        field.value_range,
    )
}

/// Parse a nominal command's `with:` and `maybe:` argument blocks.
pub(crate) fn parse_command_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<CommandBody, AnalyzeError> {
    let body = parse_concept_body(assertion, scope).map_err(|error| {
        AnalyzeError::at(
            AnalyzeErrorKind::InvalidCommandBody {
                reason: error.to_string(),
            },
            error.range.unwrap_or(assertion.range),
        )
    })?;
    if body.transient {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::InvalidCommandBody {
                reason: "`transient:` is not valid on a nominal command".into(),
            },
            assertion.range,
        ));
    }
    if body.asserts_nothing || !body.retracted.is_empty() || body.rest_retraction.is_some() {
        return Err(AnalyzeError::at(
            AnalyzeErrorKind::InvalidCommandBody {
                reason: "nominal command schemas cannot use field retraction syntax".into(),
            },
            assertion.range,
        ));
    }

    let mut schema = CommandSchema::default();
    for (name, field) in body.descriptor.with().iter() {
        if name == "this" {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidCommandBody {
                    reason: "`this` is reserved for the runtime-assigned command occurrence".into(),
                },
                assertion.range,
            ));
        }
        let descriptor = field.descriptor().clone();
        if descriptor.domain().starts_with("dom.event") {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidCommandBody {
                    reason: format!(
                        "argument {name:?} uses legacy DOM-addressed attribute {}/{}",
                        descriptor.domain(),
                        descriptor.name()
                    ),
                },
                assertion.range,
            ));
        }
        if field.is_optional() {
            schema.optional.insert(name.to_owned(), descriptor);
        } else {
            schema.required.insert(name.to_owned(), descriptor);
        }
    }
    Ok(CommandBody { schema })
}

pub(crate) fn parse_concept_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<ConceptBody, AnalyzeError> {
    let mut description: Option<String> = None;
    let mut transient: bool = false;
    // Each entry: (field name, definition, optional). `with:` fields
    // are required; `maybe:` fields are optional.
    let mut fields: Vec<(String, AttributeDefinition, bool)> = Vec::new();
    let mut inline_attributes: Vec<AttributeBody> = Vec::new();
    let mut retracted: Vec<RetractedField> = Vec::new();
    let mut rest_retraction: Option<RestRetraction> = None;
    // A concept's entity is content-derived from its descriptor by
    // default, but a `this: <uri>` pins it to a stable, chosen
    // entity (e.g. `tonk:view`) so the concept is referenceable by
    // that URI even if its published name later moves. Mirrors how
    // built-in concepts pin themselves to `db:<name>`.
    let mut pinned_entity: Option<Entity> = None;
    // Track declared field names (with their first occurrence range)
    // to reject duplicates across `with:`/`maybe:`.
    let mut seen: BTreeMap<String, lsp_types::Range> = BTreeMap::new();
    for field in &assertion.fields {
        // `..: _` is the rest-retraction marker: drop every stored
        // field of the concept. It never contributes to the
        // descriptor. `this:` is read below as the optional entity
        // pin; it likewise doesn't become a field.
        if field.name == ".." {
            if matches!(field.value, FieldValue::Blank) {
                rest_retraction = Some(RestRetraction);
            }
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
                parse_concept_field_block(
                    field,
                    false,
                    scope,
                    &mut fields,
                    &mut inline_attributes,
                    &mut retracted,
                    &mut seen,
                )?;
            }
            "maybe" => {
                parse_concept_field_block(
                    field,
                    true,
                    scope,
                    &mut fields,
                    &mut inline_attributes,
                    &mut retracted,
                    &mut seen,
                )?;
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
    // A retraction-only body (`with: { field: _ }` or `..: _`) is a
    // valid edit of an existing concept and carries no asserted
    // fields. Otherwise a concept must declare at least one
    // *required* field — an empty `with:` or a body with only
    // `maybe:` fields would constrain nothing and match every entity.
    let is_retraction_only = !retracted.is_empty() || rest_retraction.is_some();
    if !is_retraction_only && !fields.iter().any(|(_, _, optional)| !optional) {
        return Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: "`with:` is required and must declare at least one field".into(),
        }
        .into());
    }
    let mut shape = serde_json::Map::new();
    if let Some(d) = &description {
        shape.insert("description".into(), serde_json::Value::String(d.clone()));
    }
    let with_obj: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(name, attr, optional)| {
            let mut value = serde_json::to_value(&attr.descriptor)
                .expect("AttributeDescriptor is serializable");
            // The optional flag is flattened into each field object
            // on the wire; required fields omit it entirely.
            if *optional && let Some(obj) = value.as_object_mut() {
                obj.insert("optional".into(), serde_json::Value::Bool(true));
            }
            (name.clone(), value)
        })
        .collect();
    // A descriptor must carry at least one field. A retraction-only
    // body (`with: { f: _ }` / `..: _` with nothing left to assert)
    // has none — it asserts nothing, so the descriptor is a stub
    // never emitted (the emitter skips the assertion when the
    // descriptor is empty) and the entity must come from the `this:`
    // pin. Mark the stub so `with()` reports empty downstream.
    let asserts_nothing = with_obj.is_empty();
    if asserts_nothing {
        let mut stub = serde_json::Map::new();
        stub.insert(STUB_FIELD.into(), stub_attr());
        shape.insert("with".into(), serde_json::Value::Object(stub));
    } else {
        shape.insert("with".into(), serde_json::Value::Object(with_obj));
    }
    let raw_descriptor: ConceptDescriptor =
        serde_json::from_value(serde_json::Value::Object(shape)).map_err(|e| {
            AnalyzeErrorKind::InvalidConceptBody {
                reason: e.to_string(),
            }
        })?;
    // `this:` pins the entity; otherwise derive it from the
    // descriptor (content-addressed). A retraction-only body has no
    // content to derive from, so the pin is mandatory.
    let entity = match pinned_entity {
        Some(e) => e,
        None if asserts_nothing => {
            return Err(AnalyzeErrorKind::InvalidConceptBody {
                reason: "a retraction-only `concept!:` body (`field: _` / `..: _`) \
                         must pin the concept with `this: <uri>`"
                    .into(),
            }
            .into());
        }
        None => raw_descriptor.this(),
    };
    let descriptor = raw_descriptor;
    Ok(ConceptBody {
        descriptor,
        entity,
        retracted,
        rest_retraction,
        asserts_nothing,
        transient,
        inline_attributes,
    })
}

/// Placeholder field for the stub descriptor a retraction-only body
/// carries — valid (a descriptor needs ≥1 field) but never emitted.
const STUB_FIELD: &str = "_";

/// The placeholder attribute spec paired with [`STUB_FIELD`].
fn stub_attr() -> serde_json::Value {
    serde_json::json!({
        "the": "dialog.concept/stub",
        "as": "Text",
        "cardinality": "one",
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

/// Parse one `with:` or `maybe:` block of a `concept!` body. Both
/// blocks share the same field shapes (bare reference or inline
/// attribute definition); `optional` selects whether the fields are
/// required (`with:`) or set-widened (`maybe:`).
///
/// Appends `(name, definition, optional)` to `fields`, registers any
/// inline definitions in `inline_attributes`, collects `field: _`
/// retractions into `retracted`, and rejects a field name already
/// seen in either block via `seen`.
fn parse_concept_field_block(
    field: &SyntaxField,
    optional: bool,
    scope: &Scope,
    fields: &mut Vec<(String, AttributeDefinition, bool)>,
    inline_attributes: &mut Vec<AttributeBody>,
    retracted: &mut Vec<RetractedField>,
    seen: &mut BTreeMap<String, lsp_types::Range>,
) -> Result<(), AnalyzeError> {
    let block = if optional { "maybe" } else { "with" };
    let FieldValue::Nested(inner) = &field.value else {
        return Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: format!(
                "`{block}:` must be a mapping of field name → \
                 attribute reference (bare symbol, `?var`, URI) or \
                 inline attribute definition (mapping with \
                 `the`/`as`/`cardinality`/`description`)"
            ),
        }
        .into());
    };
    for sub in inner {
        // `..` is the top-level rest-retraction marker, not a field
        // name. Nested in a `with:`/`maybe:` block it would otherwise
        // be misread as a field literally named `..`; reject it with a
        // pointer to the correct placement.
        if sub.name == ".." {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidConceptBody {
                    reason: format!(
                        "`..:` is the rest-retraction marker and must be a direct \
                         child of the `concept!:` body, not nested inside `{block}:`"
                    ),
                },
                sub.name_range,
            ));
        }
        // Reject a field name declared twice (in one block or across
        // `with:`/`maybe:`) — a field is required or optional, never
        // both. Anchor the diagnostic at the second occurrence.
        if seen.insert(sub.name.clone(), sub.name_range).is_some() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::DuplicateConceptField {
                    concept: "concept".into(),
                    field: sub.name.clone(),
                },
                sub.name_range,
            ));
        }
        if matches!(sub.value, FieldValue::Blank) {
            // `field: _` retracts the named field from the stored
            // concept. It is NOT a reference and never enters the
            // descriptor — recorded separately so the emitter
            // dissociates `dialog.concept.<block>/<field>` (plus the
            // `optional` sibling) without re-asserting anything.
            retracted.push(RetractedField {
                name: sub.name.clone(),
            });
        } else if let FieldValue::Nested(attr_fields) = &sub.value {
            // Inline attribute definition. Parse it as an attribute
            // body and register it for emission as a separate
            // meta-head plan.
            let plan = parse_attribute_fields(attr_fields)?;
            let resolved = AttributeDefinition {
                entity: plan.entity.clone(),
                descriptor: plan.descriptor.clone(),
            };
            fields.push((sub.name.clone(), resolved, optional));
            inline_attributes.push(plan);
        } else {
            let resolved = resolve_concept_field(&sub.name, &sub.value, scope)?;
            fields.push((sub.name.clone(), resolved, optional));
        }
    }
    Ok(())
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
    name: Option<AnchorName>,
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
    name: Option<AnchorName>,
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
        // Optional fields carry a boolean marker claim; required
        // fields leave the term absent so no claim is emitted.
        if attr.is_optional() {
            terms.insert(
                format!("optional.{field_name}"),
                Term::Constant(Value::Boolean(true)),
            );
        }
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

/// Lower a `concept!:` body's field retractions (`with: { f: _ }`,
/// `maybe: { f: _ }`, `..: _`) into retraction [`Application`]s.
///
/// `resolved` is the concept as read off the branch (the scope's
/// prefetch). Retraction is strict: a body that asks to drop a field
/// must target a concept that exists on the branch and actually
/// carries that field — otherwise there's no `dialog.concept.with/…`
/// triple to dissociate (its value is unknown). Both cases surface
/// as [`AnalyzeErrorKind::InvalidConceptBody`].
pub(crate) fn build_concept_retractions(
    entity: &Entity,
    retracted: &[RetractedField],
    rest: Option<&RestRetraction>,
    resolved: Option<&ConceptDescriptor>,
) -> Result<Vec<Application>, AnalyzeError> {
    if retracted.is_empty() && rest.is_none() {
        return Ok(Vec::new());
    }
    let Some(stored) = resolved else {
        return Err(AnalyzeErrorKind::InvalidConceptBody {
            reason: format!(
                "field retraction (`field: _` / `..: _`) requires the concept \
                 `{entity}` to already exist on the branch, but no concept was \
                 found there"
            ),
        }
        .into());
    };

    // `..: _` drops every stored field; otherwise drop just the named
    // ones, verifying each is actually present.
    let field_names: Vec<String> = if rest.is_some() {
        stored
            .with()
            .iter()
            .map(|(name, _)| name.to_string())
            .collect()
    } else {
        for field in retracted {
            if !stored
                .with()
                .iter()
                .any(|(name, _)| name == field.name.as_str())
            {
                return Err(AnalyzeErrorKind::InvalidConceptBody {
                    reason: format!(
                        "cannot retract field `{}` from concept `{entity}`: \
                         the concept has no such field on the branch",
                        field.name
                    ),
                }
                .into());
            }
        }
        retracted.iter().map(|f| f.name.clone()).collect()
    };

    Ok(concept_field_retraction(stored, entity, &field_names)
        .into_iter()
        .collect())
}

/// Build a retraction `Application::Concept` that dissociates the
/// named `fields` from a stored concept, leaving the concept's
/// marker and every other field intact.
///
/// `stored` is the concept as resolved off the branch — it supplies
/// each retracted field's attribute `the:` so the predicate maps the
/// field to the right `dialog.concept.with/<field>` relation. The
/// field terms are emitted **blank** (`Term::blank()`): the evaluator
/// reads each as a retraction directive and queries the branch for
/// the stored value to dissociate (mirroring how instance `..: _`
/// retraction resolves its targets). Only `this` + one blank
/// `with.<field>` term per retracted field is set — no
/// `concept`/`name`/`description`/`transient` term, so a
/// `Statement::Retract` of this application touches nothing else.
///
/// Returns `None` when none of `fields` is present on the stored
/// concept (nothing to retract).
fn concept_field_retraction(
    stored: &ConceptDescriptor,
    entity: &Entity,
    fields: &[String],
) -> Option<Application> {
    // Lift exactly the fields being dropped into a sub-descriptor,
    // carrying each one's stored `ConceptFieldDescriptor` (its `the:`
    // attribute) so `concept_schema` maps it to the right
    // `dialog.concept.with/<field>` relation.
    let with = stored.with();
    let sub_fields: Vec<(String, ConceptFieldDescriptor)> = fields
        .iter()
        .filter_map(|field| {
            with.iter()
                .find(|(name, _)| name == field)
                .map(|(name, attr)| (name.to_owned(), attr.clone()))
        })
        .collect();
    if sub_fields.is_empty() {
        return None;
    }
    let sub_descriptor =
        ConceptDescriptor::try_from(sub_fields).expect("subset of a valid descriptor is valid");

    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    for (field_name, _attr) in sub_descriptor.with().iter() {
        // Blank term → the evaluator dissociates whatever value the
        // branch holds for this field (no need to recompute the
        // attribute entity analyzer-side).
        terms.insert(
            format!("with.{field_name}"),
            Term::<dialog_query::Any>::blank(),
        );
    }
    Some(Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: concept_schema(&sub_descriptor),
        },
        this: ThisIntent::Uri(entity.clone()),
        name: None,
    })
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
    for (name, attr) in descriptor.with().iter() {
        with.insert(
            format!("with.{name}"),
            serde_json::json!({
                "the": format!("dialog.concept.with/{name}"),
                "as": "Entity",
                "cardinality": "one",
            }),
        );
        // Optional fields get a sibling boolean marker field so the
        // `optional.{name}` term above is recognized by the schema.
        if attr.is_optional() {
            with.insert(
                format!("optional.{name}"),
                serde_json::json!({
                    "the": format!("dialog.concept.optional/{name}"),
                    "as": "Boolean",
                    "cardinality": "one",
                }),
            );
        }
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
        | FieldValue::Premises(_)
        | FieldValue::List(_) => {
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
