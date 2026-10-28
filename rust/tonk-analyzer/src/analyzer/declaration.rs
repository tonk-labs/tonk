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
use super::field::{is_meta_field, scalar_to_string};
use super::scope::Scope;
use tonk_core::meta::AnchorName;
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
    /// The intent role the attribute plays in its command (`role:`),
    /// kept out of the descriptor so it never changes the attribute's
    /// identity. Emitted as its own fact by [`role_application`].
    pub role: Option<String>,
}

/// The roles an attribute can play when the palette fills a command:
/// the grammar's roles, plus `now` for the nonce fields nothing typed
/// fills.
const ROLES: [&str; 10] = [
    "object",
    "goal",
    "source",
    "location",
    "time",
    "instrument",
    "format",
    "modifier",
    "alias",
    "now",
];

pub(crate) fn parse_attribute_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<AttributeBody, AnalyzeError> {
    parse_attribute_fields(&assertion.fields, scope)
}

/// Parse an attribute definition's fields into a descriptor.
///
/// Used by `attribute!:` heads and by inline
/// `with: { foo: { the: …, as: …, … } }` definitions nested
/// inside a `concept!` body. Same shape: `the`, `as`,
/// `cardinality`, and a *required* `description`.
pub(crate) fn parse_attribute_fields(
    fields: &[tonk_notation::Field],
    scope: &Scope,
) -> Result<AttributeBody, AnalyzeError> {
    let mut shape = serde_json::Map::new();
    // `as: {[position]: entity}` declares a keyed collection: the
    // bracketed key kind, and the type of each entry's value. `the:`
    // then names a domain rather than one attribute.
    let mut keyed: Option<&'static str> = None;
    let mut role: Option<String> = None;
    for field in fields {
        // `this:` and `..:` are reserved meta-keys handled by the
        // outer assertion-binding flow; they don't contribute to
        // the attribute descriptor itself.
        if is_meta_field(&field.name) {
            continue;
        }
        // Per-field value-shape requirements:
        //
        // - `as` names a type by its built-in anchor (`text`), or by
        //   the entity dialog names it by (`:text`, or the quoted
        //   `"text:"`). Lowers to that entity.
        // - `cardinality`, `pick` and `role` take a keyword
        //   (`pick: :all`); a bare symbol there is a reference and is
        //   refused.
        // - `the` accepts a URI (`xyz.tonk/foo`) or a literal
        //   string holding the same shape.
        // - `description` requires a quoted string literal —
        //   bare symbols are rejected to discourage one-word
        //   non-descriptions like `description: recipe`.
        match field.name.as_str() {
            // `as:` names a type, a keyed collection type, or lists the
            // values the attribute ranks among, best first: a list is a
            // ranked choice and reads as `top` without saying so.
            "as" if matches!(field.value, FieldValue::List(_)) => {
                let FieldValue::List(items) = &field.value else {
                    unreachable!("guarded");
                };
                let mut listed = Vec::with_capacity(items.len());
                for item in items {
                    let scalar = tonk_notation::Field {
                        name: field.name.clone(),
                        value: item.clone(),
                        name_range: field.name_range,
                        value_range: field.value_range,
                    };
                    // A listed value is a value: a bare symbol is a
                    // reference and names what its anchor names, and
                    // text is quoted.
                    let value = match &scalar.value {
                        FieldValue::Symbol(name) => scope
                            .symbol(name)
                            .map(|entity| entity.to_string())
                            .ok_or_else(|| {
                                AnalyzeError::at(
                                    AnalyzeErrorKind::UnknownNameReference {
                                        field: field.name.clone(),
                                        name: name.clone(),
                                    },
                                    field.value_range,
                                )
                            })?,
                        _ => stringify_simple_value(&scalar)?,
                    };
                    listed.push(serde_json::Value::String(value));
                }
                shape.insert("as".into(), serde_json::Value::Array(listed));
            }
            "as" => {
                let value_field = match &field.value {
                    FieldValue::Nested(inner) => {
                        let [entry] = inner.as_slice() else {
                            return Err(AnalyzeErrorKind::InvalidAttributeBody {
                                reason: "a keyed collection type is one entry: \
                                         `{[position]: <type>}` or `{[symbol]: <type>}`"
                                    .into(),
                            }
                            .into());
                        };
                        keyed = Some(match entry.name.as_str() {
                            "[position]" => "sequence",
                            "[symbol]" => "dictionary",
                            other => {
                                return Err(AnalyzeErrorKind::InvalidAttributeBody {
                                    reason: format!(
                                        "unknown key kind {other:?} — expected \
                                         `[position]` (a sequence) or `[symbol]` \
                                         (a dictionary)"
                                    ),
                                }
                                .into());
                            }
                        });
                        entry
                    }
                    _ => field,
                };
                let kind = attribute_type(value_field, scope)?;
                shape.insert("as".into(), serde_json::Value::String(kind));
            }
            // The older spelling of a pick: `one` is `pick: last`, the
            // default, and `many` is `pick: all`. Accepted so existing
            // documents lower; a `pick:` beside it wins.
            "cardinality" => {
                let value_str = keyword_of(field, "many")?;
                let normalized = normalize_cardinality_name(&value_str).ok_or_else(|| {
                    AnalyzeErrorKind::InvalidAttributeBody {
                        reason: format!(
                            "unknown cardinality {value_str:?} — \
                             expected `:one` or `:many`; prefer `pick: :last` or `pick: :all`"
                        ),
                    }
                })?;
                if normalized == "many" && !shape.contains_key("pick") {
                    shape.insert(
                        "pick".into(),
                        serde_json::Value::String(tonk_notation::Pick::All.uri().into()),
                    );
                }
            }
            // `the:` names the relation, or lists relations best first:
            // a ranked choice by relation, read as `top`.
            "the" if matches!(field.value, FieldValue::List(_)) => {
                let FieldValue::List(items) = &field.value else {
                    unreachable!("guarded");
                };
                let mut listed = Vec::with_capacity(items.len());
                for item in items {
                    let scalar = tonk_notation::Field {
                        name: field.name.clone(),
                        value: item.clone(),
                        name_range: field.name_range,
                        value_range: field.value_range,
                    };
                    listed.push(serde_json::Value::String(stringify_simple_value(&scalar)?));
                }
                shape.insert("the".into(), serde_json::Value::Array(listed));
            }
            "the" => {
                let value_str = stringify_simple_value(field)?;
                shape.insert("the".into(), serde_json::Value::String(value_str));
            }
            // Which of its relation's candidates a field reads (dialog's
            // `Pick`): `last` (the default), `all`, `top` (what a listed
            // `as:` or `the:` implies), `max`, `min`. Every pick keeps
            // members of the candidate set; folds (sums, counts) belong
            // to `reduce` in queries. Dialog checks the pick against the
            // attribute when the concept is built.
            "pick" => {
                let keyword = keyword_of(field, "all")?;
                let pick = tonk_notation::Pick::from_anchor(&keyword).ok_or_else(|| {
                    AnalyzeErrorKind::InvalidAttributeBody {
                        reason: format!(
                            "unknown pick {keyword:?} — expected one of: {}",
                            tonk_notation::Pick::ALL
                                .iter()
                                .map(|pick| format!(":{}", pick.anchor()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }
                })?;
                shape.insert("pick".into(), serde_json::Value::String(pick.uri().into()));
            }
            "select" => {
                return Err(AnalyzeErrorKind::InvalidAttributeBody {
                    reason: "`select:` is now spelled `pick:`".into(),
                }
                .into());
            }
            "description" => {
                let value_str = require_string_description(field)?;
                shape.insert("description".into(), serde_json::Value::String(value_str));
            }
            "role" => {
                let value_str = keyword_of(field, "object")?;
                if !ROLES.contains(&value_str.as_str()) {
                    return Err(AnalyzeErrorKind::InvalidAttributeBody {
                        reason: format!(
                            "unknown role {value_str:?} — expected one of: {}",
                            ROLES.join(", ")
                        ),
                    }
                    .into());
                }
                role = Some(value_str);
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
    if let Some(keyed) = keyed {
        let the = shape["the"].as_str().unwrap_or_default().to_owned();
        if the.is_empty() {
            return Err(AnalyzeErrorKind::InvalidAttributeBody {
                reason: "a keyed collection reads one relation, not a ranked list".into(),
            }
            .into());
        }
        if the.contains('/') {
            return Err(AnalyzeErrorKind::InvalidAttributeBody {
                reason: format!(
                    "a keyed collection's `the:` names a domain, not an \
                     attribute — write `the: {}` and let the key supply the \
                     name half",
                    the.split('/').next().unwrap_or_default()
                ),
            }
            .into());
        }
        shape.insert(
            "the".into(),
            serde_json::json!({ "domain": the, "keyed": keyed }),
        );
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
    Ok(AttributeBody {
        descriptor,
        entity,
        role,
    })
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
    /// Typed edge bookmarks; kept outside the ordinary matching descriptor.
    pub edges: BTreeMap<String, String>,
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
    /// needed to emit `db.attribute/{id,type,cardinality}`
    /// and `db.meta/description` claims so the attribute is
    /// queryable via `attribute:` after the `concept!` commits.
    pub inline_attributes: Vec<AttributeBody>,
    /// The command's names in the palette (`action:`, one name or a
    /// list), each lowered to an `intent/action` fact on the concept by
    /// [`action_application`].
    pub action: Vec<String>,
}

pub(crate) fn parse_concept_body(
    assertion: &SyntaxApplication,
    scope: &Scope,
) -> Result<ConceptBody, AnalyzeError> {
    // Normalize typed fields to entity-valued attributes, retaining their
    // targets separately. Bookmarks deliberately need not resolve yet: a
    // declaration may refer to itself or to a later declaration.
    let mut assertion = assertion.clone();
    let mut edges = BTreeMap::new();
    for block in &mut assertion.fields {
        if !matches!(block.name.as_str(), "with" | "maybe") {
            continue;
        }
        let FieldValue::Nested(fields) = &mut block.value else {
            continue;
        };
        for field in fields {
            let FieldValue::Nested(properties) = &mut field.value else {
                continue;
            };
            for property in properties {
                if property.name != "as" {
                    continue;
                }
                let target = match &property.value {
                    FieldValue::Symbol(name)
                        if tonk_notation::type_uri(name).is_none()
                            || scope.shadowed_builtins.lock().contains(name) =>
                    {
                        Some(name.clone())
                    }
                    FieldValue::Uri(uri) if tonk_notation::type_uri(uri).is_none() => {
                        Some(uri.clone())
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    edges.insert(field.name.clone(), target);
                    property.value = FieldValue::Symbol("entity".into());
                }
            }
        }
    }
    let mut description: Option<String> = None;
    let mut transient: bool = false;
    let mut action: Vec<String> = Vec::new();
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
            // An action name is text the intent layer matches, so it is
            // quoted: a bare word is a reference, and an action names
            // no entity.
            "action" => {
                let bare = match &field.value {
                    FieldValue::Symbol(name) => Some(name),
                    FieldValue::List(items) => items.iter().find_map(|item| match item {
                        FieldValue::Symbol(name) => Some(name),
                        _ => None,
                    }),
                    _ => None,
                };
                if let Some(name) = bare {
                    return Err(AnalyzeErrorKind::InvalidConceptBody {
                        reason: format!(
                            "`action:` takes the command's name as text; quote it: `\"{name}\"`"
                        ),
                    }
                    .into());
                }
                let names = match &field.value {
                    FieldValue::List(items) => items
                        .iter()
                        .map(|value| {
                            stringify_simple_value(&tonk_notation::Field {
                                value: value.clone(),
                                ..field.clone()
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => vec![stringify_simple_value(field)?],
                };
                if names.is_empty() || names.iter().any(|name| name.trim().is_empty()) {
                    return Err(AnalyzeErrorKind::InvalidConceptBody {
                        reason: "`action:` names the command; a name can't be empty".into(),
                    }
                    .into());
                }
                action = names;
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
        None if edges.is_empty() => raw_descriptor.this(),
        None => {
            use tonk_schema::prelude::EntityExt;
            Entity::of(&("tonk.graph.concept", &raw_descriptor, &edges))
        }
    };
    let descriptor = raw_descriptor;
    Ok(ConceptBody {
        edges,
        descriptor,
        entity,
        retracted,
        rest_retraction,
        asserts_nothing,
        transient,
        inline_attributes,
        action,
    })
}

/// Placeholder field for the stub descriptor a retraction-only body
/// carries — valid (a descriptor needs ≥1 field) but never emitted.
const STUB_FIELD: &str = "_";

/// The placeholder attribute spec paired with [`STUB_FIELD`].
fn stub_attr() -> serde_json::Value {
    serde_json::json!({
        "the": "db.concept/stub",
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
            let entity = uri.parse().map_err(|e: dialog_artifacts::IdentityError| {
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
        // An attribute body's own keys directly under `with:`/`maybe:`
        // are an inline definition missing its field name: `the:` and
        // `as:` are no concept field's name. Said so, rather than read
        // as a field named `the` or `as` referring to an attribute,
        // which fails on the relation not being an entity URI, or on
        // the type entity not being an attribute.
        if matches!(sub.name.as_str(), "the" | "as") {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::InvalidConceptBody {
                    reason: format!(
                        "`{name}:` directly under `{block}:` is an attribute body without a \
                         field name — `{block}:` maps each field name to its attribute, so \
                         nest the body under the field: `{block}:` / `<field>:` / `{name}: …`",
                        name = sub.name
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
            // dissociates `db.concept.<block>/<field>` (plus the
            // `optional` sibling) without re-asserting anything.
            retracted.push(RetractedField {
                name: sub.name.clone(),
            });
        } else if let FieldValue::Nested(attr_fields) = &sub.value {
            // Inline attribute definition. Parse it as an attribute
            // body and register it for emission as a separate
            // meta-head plan.
            let plan = parse_attribute_fields(attr_fields, scope)?;
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
            // A field's value here is the attribute it reads, by its
            // entity. A relation (`io.gozala.person/name`) is not one,
            // and neither is a type (`:text`): both are keys of the
            // attribute body the field would declare inline instead.
            let Ok(entity) = uri.parse::<Entity>() else {
                return Err(AnalyzeErrorKind::InvalidConceptBody {
                    reason: format!(
                        "field {field_name:?} refers to `{uri}`, which is a relation, not an \
                         attribute entity — name a declared attribute (its `&anchor`), or \
                         declare the attribute inline: `{field_name}:` / `the: {uri}` / \
                         `as: …` / `description: …`"
                    ),
                }
                .into());
            };
            if tonk_notation::type_uri(uri).is_some() {
                return Err(AnalyzeErrorKind::InvalidConceptBody {
                    reason: format!(
                        "field {field_name:?} refers to `{uri}`, which is a type, not an \
                         attribute — a field's type is declared in its attribute body: \
                         `{field_name}:` / `the: <relation>` / `as: {uri}` / `description: …`"
                    ),
                }
                .into());
            }
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
/// description. The published name (`db.meta/name` claim on
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
    // `domain/name` for an attribute; a collection spells its key
    // kind in the name slot, `domain/[position]`, which is how the
    // read side tells the two apart.
    terms.insert(
        "id".into(),
        Term::Constant(Value::String(descriptor.the().to_string())),
    );
    // The value type, as the entity dialog names it (`text:`), when
    // the attribute has one.
    if let Some(kind) = descriptor
        .content_type()
        .and_then(|kind| kind.uri().parse::<Entity>().ok())
    {
        terms.insert("as".into(), Term::Constant(Value::Entity(kind)));
    }
    terms.insert(
        "description".into(),
        Term::Constant(Value::String(descriptor.description().to_owned())),
    );
    // The pick the attribute reads under, as the entity dialog names
    // it (`all:`), and the values its `as:` lists as a JSON list, best
    // first: a reader reconstructing the descriptor needs both.
    if let Ok(pick) = descriptor.pick().uri().parse::<Entity>() {
        terms.insert("pick".into(), Term::Constant(Value::Entity(pick)));
    }
    if !descriptor.among().is_empty()
        && let Ok(among) = serde_json::to_string(descriptor.among())
    {
        terms.insert("among".into(), Term::Constant(Value::String(among)));
    }
    Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: attribute_schema(),
        },
        join: Vec::new(),
        this: ThisIntent::Uri(entity.clone()),
        name,
    }
}

/// Build the `Application` for a `concept!` head — the asserted
/// predicate is a synthesized concept-of-concept schema (one
/// `with.<field>` per field of the user's concept, plus the
/// `db.meta/concept` marker and `db.meta/description`).
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
    // `transient: true` adds dialog's `(this,
    // dialog.concept/transient, true)` marker fact. The
    // synthesized `concept_schema` includes a matching field;
    // durable concepts skip the term so no claim is emitted (the
    // emitter ignores fields whose term is absent).
    if transient {
        terms.insert("transient".into(), Term::Constant(Value::Boolean(true)));
    }
    Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: concept_schema(descriptor),
        },
        join: Vec::new(),
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
/// carries that field — otherwise there's no `db.concept.with/…`
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
/// field to the right `db.concept.with/<field>` relation. The
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
    // `db.concept.with/<field>` relation.
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
        join: Vec::new(),
        this: ThisIntent::Uri(entity.clone()),
        name: None,
    })
}

/// Build the `dialog.attribute` built-in schema descriptor. Its
/// fields map to the 5 EAVs every named attribute writes.
/// Assert or clear one typed edge bookmark on a concept.
pub(crate) fn edge_application(entity: &Entity, field: &str, target: Option<&str>) -> Application {
    let predicate: ConceptDescriptor = serde_json::from_value(serde_json::json!({
        "with": { "target": { "the": format!("db.concept.edge/{field}"), "as": "text:" } }
    }))
    .expect("edge metadata schema is valid");
    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    terms.insert(
        "target".into(),
        target
            .map(|target| Term::Constant(Value::String(target.into())))
            .unwrap_or_else(Term::blank),
    );
    Application::Concept {
        query: ConceptQuery { terms, predicate },
        join: Vec::new(),
        this: ThisIntent::Uri(entity.clone()),
        name: None,
    }
}

/// Build the `Application` asserting an attribute's intent role: one
/// fact, `tonk.dialog.intent.attribute/role`, on the attribute entity.
pub(crate) fn role_application(entity: &Entity, role: &str) -> Application {
    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    terms.insert(
        "role".into(),
        Term::Constant(Value::String(role.to_owned())),
    );
    Application::Concept {
        query: ConceptQuery {
            terms,
            predicate: role_schema(),
        },
        join: Vec::new(),
        this: ThisIntent::Uri(entity.clone()),
        name: None,
    }
}

/// Build the `Application` asserting a command's palette name: one
/// `tonk.dialog.intent.action/name` fact on the command's concept.
pub(crate) fn action_application(entity: &Entity, name: &str) -> Application {
    let mut terms = Parameters::new();
    terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
    terms.insert(
        "name".into(),
        Term::Constant(Value::String(name.to_owned())),
    );
    let predicate: ConceptDescriptor = serde_json::from_value(serde_json::json!({
        "with": {
            "name": { "the": "tonk.dialog.intent.action/name", "as": "Text", "cardinality": "many" },
        }
    }))
    .expect("action schema is well-formed");
    Application::Concept {
        query: ConceptQuery { terms, predicate },
        join: Vec::new(),
        this: ThisIntent::Uri(entity.clone()),
        name: None,
    }
}

fn role_schema() -> ConceptDescriptor {
    let json = serde_json::json!({
        "with": {
            "role": { "the": "tonk.dialog.intent.attribute/role", "as": "Text", "cardinality": "one" },
        }
    });
    serde_json::from_value(json).expect("role schema is well-formed")
}

fn attribute_schema() -> ConceptDescriptor {
    let json = serde_json::json!({
        "with": {
            "id":          { "the": "db.attribute/id",          "as": "text:" },
            "as":          { "the": "db.attribute/as",          "as": "entity:" },
            "pick":        { "the": "db.attribute/pick",        "as": "entity:" },
            "description": { "the": "db.meta/description",      "as": "text:" },
            "name":        { "the": "db.meta/name",             "as": "text:" },
            // The values an attribute's `as:` lists, a JSON list best
            // first. Written only by an attribute that lists them;
            // read back by `tonk_schema::concept::AttributeByEntity`.
            "among":       { "the": "db.attribute/among",       "as": "text:" },
        }
    });
    serde_json::from_value(json).expect("attribute schema is well-formed")
}

/// Build a `concept!` schema descriptor — one `with.<field>` per
/// field of the concept being defined, plus the
/// `db.meta/concept` marker (so branch-wide `concept:` queries
/// can find every concept entity) and optional name and
/// description fields.
fn concept_schema(descriptor: &ConceptDescriptor) -> ConceptDescriptor {
    let mut with = serde_json::Map::new();
    for (name, attr) in descriptor.with().iter() {
        with.insert(
            format!("with.{name}"),
            serde_json::json!({
                "the": format!("db.concept.with/{name}"),
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
                    "the": format!("db.concept.optional/{name}"),
                    "as": "Boolean",
                    "cardinality": "one",
                }),
            );
        }
    }
    with.insert(
        "concept".into(),
        serde_json::json!({
            "the": "db.meta/concept",
            "as": "Entity",
            "cardinality": "one",
        }),
    );
    with.insert(
        "name".into(),
        serde_json::json!({
            "the": "db.meta/name",
            "as": "Text",
            "cardinality": "one",
        }),
    );
    with.insert(
        "description".into(),
        serde_json::json!({
            "the": "db.meta/description",
            "as": "Text",
            "cardinality": "one",
        }),
    );
    with.insert(
        "transient".into(),
        serde_json::json!({
            "the": "dialog.concept/transient",
            "as": "Boolean",
            "cardinality": "one",
        }),
    );
    serde_json::from_value(serde_json::json!({ "with": with }))
        .expect("concept schema is well-formed")
}

/// The type an attribute's `as:` names, as the entity dialog names it
/// by: a built-in type anchor (`text`, `integer`, `natural`, ... and
/// the aliases `signed-integer` and `unsigned-integer`), a sized number
/// type (`natural:64`, `float:32`), `number` (every number), or, quoted,
/// the entity itself (`"text:"`) or the name an earlier release wrote
/// (`"Text"`). An anchor the document declares under a type's name means
/// the declaration, which is no type.
fn attribute_type(field: &tonk_notation::Field, scope: &Scope) -> Result<String, AnalyzeError> {
    use tonk_notation::{ValueType, type_uri};
    let expected = || {
        ValueType::ALL
            .iter()
            .map(|kind| kind.anchor())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let kind = match &field.value {
        FieldValue::Symbol(name) => {
            if scope.shadowed_builtins.lock().contains(name) {
                return Err(AnalyzeErrorKind::InvalidAttributeBody {
                    reason: format!(
                        "`{name}` names this document's own anchor `&{name}`, \
                         not the built-in type; rename the anchor to use the type"
                    ),
                }
                .into());
            }
            type_uri(name)
        }
        _ => type_uri(&stringify_simple_value(field)?),
    };
    kind.ok_or_else(|| {
        AnalyzeErrorKind::InvalidAttributeBody {
            reason: format!(
                "unknown attribute type {:?} — expected one of: {}",
                match &field.value {
                    FieldValue::Symbol(name) => name.clone(),
                    _ => stringify_simple_value(field).unwrap_or_default(),
                },
                expected()
            ),
        }
        .into()
    })
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

/// Coerce a "simple" declaration field into a string. Used for
/// `the:`, a quoted `as:` and `action:` — slots whose value is a URI
/// or a literal. Bare symbols (references), variables, blanks, and
/// nested mappings are rejected.
fn stringify_simple_value(field: &tonk_notation::Field) -> Result<String, AnalyzeError> {
    Ok(match &field.value {
        FieldValue::Literal(Scalar::String(s)) => s.clone(),
        FieldValue::Literal(other) => scalar_to_string(other)?,
        FieldValue::Uri(s) => s.clone(),
        // A bare symbol is a reference to an anchor, never text: a
        // relation is written as a URI (`xyz.tonk.task/title`) and a
        // name as a quoted string.
        FieldValue::Symbol(s) => {
            return Err(AnalyzeErrorKind::InvalidAttributeBody {
                reason: format!(
                    "`{field}: {s}` reads `{s}` as a reference to an anchor; \
                     write a URI or a quoted string (`{field}: \"{s}\"`)",
                    field = field.name
                ),
            }
            .into());
        }
        FieldValue::Include(include) => {
            return Err(
                super::field::unexpanded_include(include, None).with_range(field.value_range)
            );
        }
        FieldValue::List(_) => {
            return Err(AnalyzeErrorKind::UnsupportedFieldValue {
                field: field.name.clone(),
                form: "a list (only an attribute's `the:` or `as:` and a command's `action:` take one)",
            }
            .into());
        }
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

/// The keyword a keyword-valued field holds: `pick: :all` is `all`.
///
/// A pick, a cardinality or a role is a fixed word, never an entity,
/// so the field also reads the spellings earlier releases wrote, the
/// bare word (`pick: all`) and the quoted word (`cardinality: "one"`):
/// the templates and notebooks those releases published are written
/// that way, and a release that still reads them cannot write the
/// keyword. `example` is the keyword the message suggests when the
/// value is none of these.
fn keyword_of(field: &tonk_notation::Field, example: &str) -> Result<String, AnalyzeError> {
    let name = &field.name;
    match &field.value {
        FieldValue::Uri(uri) => match uri.strip_suffix(':') {
            Some(keyword) if !keyword.is_empty() && !keyword.contains(':') => {
                Ok(keyword.to_owned())
            }
            _ => Err(AnalyzeErrorKind::InvalidAttributeBody {
                reason: format!("`{name}:` takes a keyword such as `:{example}`, not {uri:?}"),
            }
            .into()),
        },
        FieldValue::Symbol(word) | FieldValue::Literal(Scalar::String(word)) => Ok(word.clone()),
        _ => Err(AnalyzeErrorKind::InvalidAttributeBody {
            reason: format!("`{name}:` takes a keyword such as `:{example}`"),
        }
        .into()),
    }
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
        // Prose kept in its own file is still prose.
        FieldValue::Literal(Scalar::Included(bytes)) => super::field::included_text(bytes),
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

#[cfg(test)]
mod graph_tests {
    use super::*;

    fn body(source: &str) -> ConceptBody {
        let parsed = tonk_notation::parse(source).syntax.expect("valid notation");
        parse_concept_body(parsed.expressions[0].application(), &Scope::new())
            .expect("valid declaration")
    }

    #[dialog_common::test]
    fn typed_edges_allow_self_reference_and_preserve_storage_types() {
        let graph = body(
            r#"concept!: &person
  with:
    name: { the: example.person/name, as: text, description: "Name" }
  maybe:
    manager: { the: example.person/manager, as: person, description: "Manager" }
"#,
        );
        assert_eq!(
            graph.edges.get("manager").map(String::as_str),
            Some("person")
        );
        let with = graph.descriptor.with();
        let manager = with.iter().find(|(name, _)| *name == "manager").unwrap().1;
        assert_eq!(manager.content_type(), Some(dialog_query::Type::Entity));
        assert!(manager.is_optional());
    }

    #[dialog_common::test]
    fn edge_target_changes_identity_but_ordinary_declarations_keep_theirs() {
        let ordinary = body(
            r#"concept!: &person
  with:
    manager: { the: example.person/manager, as: entity, description: "Manager" }
"#,
        );
        let first = body(
            r#"concept!: &person
  with:
    manager: { the: example.person/manager, as: person, description: "Manager" }
"#,
        );
        let second = body(
            r#"concept!: &person
  with:
    manager: { the: example.person/manager, as: team, description: "Manager" }
"#,
        );
        assert_eq!(ordinary.entity, ordinary.descriptor.this());
        assert_eq!(first.descriptor, ordinary.descriptor);
        assert_ne!(first.entity, ordinary.entity);
        assert_ne!(first.entity, second.entity);
    }
}
