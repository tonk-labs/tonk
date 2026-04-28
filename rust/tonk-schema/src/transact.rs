//! Parser for the three-level transaction notation.
//!
//! A transaction document is a map from *subject* to *context* to
//! *fields*:
//!
//! ```yaml
//! person-name:                     # level 1 — subject
//!   attribute:                     # level 2 — context
//!     description: The person's name
//!     the:         io.gozala.person/name
//!     as:          Text
//!     cardinality: one
//!
//! _:                               # anonymous subject
//!   xyz.tonk.demo:                 # domain context (contains `.`)
//!     foo: 1
//!     bar: 2
//!
//! person:
//!   concept:
//!     description: A person
//!     with:
//!       name: person-name          # bookmark reference
//!       age:  person-age
//! ```
//!
//! - **Subject** (level 1): `_` is a fresh anonymous entity; a
//!   string containing `:` is treated as a URI literal (`did:…`,
//!   `the:…`, `concept:…`); anything else is a *bookmark name* —
//!   resolved within the document and persisted to the branch as a
//!   `dialog.meta/name` claim on the entity it ends up identifying.
//!
//! - **Context** (level 2): a key containing `.` is a *domain* —
//!   each level-3 field expands to a raw `domain/{field}` claim. A
//!   key without `.` is a *concept name*. v1 understands the two
//!   built-in concepts `attribute` and `concept`; user-defined
//!   concepts are deferred.
//!
//! - **Fields** (level 3): in a domain context fields are scalar
//!   primitive values. In a concept context fields follow that
//!   concept's schema; values may be strings (bookmark or URI
//!   references) or maps (inline concept assertions).
//!
//! The parser produces a [`Transaction`] — a flat list of EAV
//! [`Claim`]s plus a `bookmarks` map mapping each named subject in
//! the document back to its resolved [`Entity`]. Callers write the
//! claims to a branch in a single transaction and return the
//! bookmark map to the requester so they can refer to the named
//! entities later.

use std::collections::BTreeMap;

use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_common::Blake3Hash;
use dialog_query::{AttributeDescriptor, ConceptDescriptor};
use serde_json::Value as Json;
use thiserror::Error;

use crate::concept;

/// A single EAV claim ready to be asserted.
#[derive(Debug, Clone)]
pub struct Claim {
    /// The relation (`domain/name`).
    pub the: ArtifactsAttribute,
    /// The entity the claim is about.
    pub of: Entity,
    /// The value being associated.
    pub is: Value,
}

/// Result of parsing a transaction document.
#[derive(Debug, Default)]
pub struct Transaction {
    /// EAV claims to be asserted in commit order.
    pub claims: Vec<Claim>,
    /// Bookmark name → resolved entity, for each named subject in
    /// the document.
    pub bookmarks: BTreeMap<String, Entity>,
}

/// Errors raised while parsing a transaction document.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The top-level document was not a map of `subject -> context`.
    #[error("transaction document must be a JSON/YAML object at the top level")]
    NotAnObject,
    /// A subject's body was not a map of `context -> fields`.
    #[error("subject {subject:?} must map to an object of `context: fields`")]
    SubjectNotAnObject {
        /// The offending subject key.
        subject: String,
    },
    /// A context block's body was not a map of `field -> value`.
    #[error("context {context:?} on subject {subject:?} must map to an object of `field: value`")]
    ContextNotAnObject {
        /// The subject containing the context.
        subject: String,
        /// The offending context key.
        context: String,
    },
    /// A subject string was not a syntactically valid URI even
    /// though it contained `:`.
    #[error("subject {subject:?} contains `:` but is not a valid URI: {reason}")]
    InvalidUri {
        /// The offending subject.
        subject: String,
        /// Underlying parse error.
        reason: String,
    },
    /// A relation could not be constructed for an EAV write.
    #[error("invalid relation {relation:?}: {reason}")]
    InvalidRelation {
        /// The offending relation string.
        relation: String,
        /// Underlying error.
        reason: String,
    },
    /// A built-in concept block (`attribute` or `concept`) failed
    /// to deserialize into the corresponding dialog descriptor.
    #[error("failed to parse {kind} block on subject {subject:?}: {reason}")]
    InvalidDescriptor {
        /// The built-in concept name.
        kind: &'static str,
        /// The subject the block was under.
        subject: String,
        /// Underlying serde error.
        reason: String,
    },
    /// A `with` or `maybe` field in a concept block referenced a
    /// bookmark that wasn't defined in the document.
    #[error("concept on subject {subject:?} references unknown bookmark {bookmark:?}")]
    UnknownBookmark {
        /// The subject containing the reference.
        subject: String,
        /// The unresolved bookmark name.
        bookmark: String,
    },
    /// A `with` or `maybe` field in a concept block referenced a
    /// bookmark that resolved to a non-attribute entity (or to one
    /// whose descriptor we couldn't reconstruct).
    #[error("concept on subject {subject:?} references {bookmark:?}, which is not an attribute")]
    NotAnAttribute {
        /// The subject containing the reference.
        subject: String,
        /// The bookmark or URI that resolved to a non-attribute.
        bookmark: String,
    },
    /// A user-defined concept name appeared as a level-2 context.
    /// User concepts require a branch lookup and are deferred to
    /// a later iteration.
    #[error(
        "context {context:?} on subject {subject:?} is not `attribute`, `concept`, or a domain (contains `.`); user-defined concepts are not yet supported"
    )]
    UnsupportedContext {
        /// The subject the context appeared on.
        subject: String,
        /// The unrecognized context key.
        context: String,
    },
    /// A nested map appeared in a domain-context field. Nested
    /// entities under a domain context are deferred to a later
    /// iteration.
    #[error("nested object under domain context {context:?}.{field:?} is not yet supported")]
    NestedNotSupported {
        /// The domain.
        context: String,
        /// The field whose value was a nested map.
        field: String,
    },
    /// The level-1 subject `_` (anonymous) is not supported. Use
    /// a bookmark name or a URI instead.
    #[error("`_` (anonymous) subject is not supported; use a bookmark name or a URI")]
    AnonymousSubjectNotSupported,
}

/// Parse a transaction document presented as JSON.
pub fn parse_json(input: &str) -> Result<Transaction, ParseError> {
    let value: Json = serde_json::from_str(input).map_err(|e| ParseError::InvalidDescriptor {
        kind: "document",
        subject: String::new(),
        reason: e.to_string(),
    })?;
    parse_value(value)
}

/// Parse a transaction document presented as YAML.
///
/// The YAML is converted to the equivalent JSON shape internally
/// so the rest of the parser only needs to walk one tree type.
pub fn parse_yaml(input: &str) -> Result<Transaction, ParseError> {
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(input).map_err(|e| ParseError::InvalidDescriptor {
            kind: "document",
            subject: String::new(),
            reason: e.to_string(),
        })?;
    let json = yaml_to_json(yaml).map_err(|e| ParseError::InvalidDescriptor {
        kind: "document",
        subject: String::new(),
        reason: e,
    })?;
    parse_value(json)
}

/// Parse a transaction document from a pre-decoded JSON value.
///
/// Useful when the input is already a `serde_json::Value` (e.g.
/// because the caller has done its own format sniffing).
pub fn parse_value(value: Json) -> Result<Transaction, ParseError> {
    let Json::Object(top) = value else {
        return Err(ParseError::NotAnObject);
    };

    let mut tx = Transaction::default();

    for (subject_key, subject_body) in top {
        let Json::Object(contexts) = subject_body else {
            return Err(ParseError::SubjectNotAnObject {
                subject: subject_key,
            });
        };

        let subject_kind = classify_subject(&subject_key)?;

        // Each subject can carry multiple contexts; they share the
        // resolved entity. The first context that produces an
        // entity wins; subsequent contexts on the same subject
        // build claims against it.
        let mut entity: Option<Entity> = None;

        for (context_key, fields) in contexts {
            let Json::Object(field_map) = fields else {
                return Err(ParseError::ContextNotAnObject {
                    subject: subject_key.clone(),
                    context: context_key,
                });
            };

            let context = classify_context(&context_key);

            let resolved = match context {
                Context::Domain => {
                    // Domain context: the subject is the entity.
                    let e = subject_kind.materialize(&subject_key, entity.as_ref())?;
                    emit_domain_claims(&context_key, &field_map, &e, &mut tx.claims)?;
                    e
                }
                Context::Attribute => emit_attribute(&subject_key, field_map, &mut tx.claims)?,
                Context::Concept => {
                    emit_concept(&subject_key, field_map, &tx.bookmarks, &mut tx.claims)?
                }
                Context::Unknown => {
                    return Err(ParseError::UnsupportedContext {
                        subject: subject_key.clone(),
                        context: context_key,
                    });
                }
            };

            entity = Some(resolved);
        }

        // Persist the bookmark, if the subject was a bare name
        // (not `_`, not a URI).
        if let SubjectKind::Bookmark = subject_kind
            && let Some(e) = entity.clone()
        {
            emit_bookmark_name(&subject_key, &e, &mut tx.claims)?;
            tx.bookmarks.insert(subject_key, e);
        }
    }

    Ok(tx)
}

#[derive(Clone, Copy)]
enum SubjectKind {
    Uri,
    Bookmark,
}

impl SubjectKind {
    fn materialize(self, subject: &str, existing: Option<&Entity>) -> Result<Entity, ParseError> {
        if let Some(e) = existing {
            return Ok(e.clone());
        }
        match self {
            SubjectKind::Uri => {
                subject
                    .parse()
                    .map_err(
                        |e: dialog_artifacts::DialogArtifactsError| ParseError::InvalidUri {
                            subject: subject.to_owned(),
                            reason: e.to_string(),
                        },
                    )
            }
            SubjectKind::Bookmark => Ok(derive_entity(subject)),
        }
    }
}

fn classify_subject(subject: &str) -> Result<SubjectKind, ParseError> {
    if subject == "_" {
        Err(ParseError::AnonymousSubjectNotSupported)
    } else if subject.contains(':') {
        Ok(SubjectKind::Uri)
    } else {
        Ok(SubjectKind::Bookmark)
    }
}

enum Context {
    Domain,
    Attribute,
    Concept,
    Unknown,
}

fn classify_context(context: &str) -> Context {
    if context.contains('.') {
        Context::Domain
    } else {
        match context {
            "attribute" => Context::Attribute,
            "concept" => Context::Concept,
            _ => Context::Unknown,
        }
    }
}

/// Resolve a bookmark name to its `did:key:z…` entity URI.
///
/// The hash of the name bytes is encoded directly as a `did:key`
/// with the ed25519 multicodec prefix (`0xed 0x01`). This matches
/// the *URI shape* carry uses for `derive_entity(name)` but skips
/// the ed25519 keypair derivation step — we encode the blake3 hash
/// bytes as if they were an ed25519 verifying key, which is
/// permitted by dialog (it does not validate the multicodec
/// payload as real key material).
///
/// The trade-off: same name produces a different `did:key:z…`
/// URI than carry's CLI would compute, so domain-context bookmarks
/// don't round-trip with carry data on disk. Fixing that requires
/// async access to dialog-credentials' ed25519 implementation,
/// which conflicts with the parser staying sync. Carry will need
/// to be migrated to this scheme as part of the broader port.
fn derive_entity(name: &str) -> Entity {
    let hash = Blake3Hash::hash(name.as_bytes());
    const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];
    let mut multicodec_bytes = [0u8; 34];
    multicodec_bytes[..2].copy_from_slice(&ED25519_MULTICODEC);
    multicodec_bytes[2..].copy_from_slice(hash.as_bytes());
    let encoded = bs58::encode(&multicodec_bytes).into_string();
    format!("did:key:z{encoded}")
        .parse()
        .expect("did:key URI built from a 34-byte multicodec payload is always valid")
}

fn emit_domain_claims(
    domain: &str,
    fields: &serde_json::Map<String, Json>,
    entity: &Entity,
    out: &mut Vec<Claim>,
) -> Result<(), ParseError> {
    for (field, value) in fields {
        if value.is_object() || value.is_array() {
            return Err(ParseError::NestedNotSupported {
                context: domain.to_owned(),
                field: field.clone(),
            });
        }
        let relation = make_relation(domain, field)?;
        out.push(Claim {
            the: relation,
            of: entity.clone(),
            is: json_scalar_to_value(value),
        });
    }
    Ok(())
}

fn emit_attribute(
    subject: &str,
    fields: serde_json::Map<String, Json>,
    out: &mut Vec<Claim>,
) -> Result<Entity, ParseError> {
    let descriptor: AttributeDescriptor =
        serde_json::from_value(Json::Object(fields)).map_err(|e| {
            ParseError::InvalidDescriptor {
                kind: "attribute",
                subject: subject.to_owned(),
                reason: e.to_string(),
            }
        })?;
    let entity: Entity = descriptor
        .to_uri()
        .parse()
        .expect("descriptor URI is always valid");

    push_descriptor_claims(&descriptor, &entity, out)?;
    Ok(entity)
}

fn emit_concept(
    subject: &str,
    mut fields: serde_json::Map<String, Json>,
    bookmarks: &BTreeMap<String, Entity>,
    out: &mut Vec<Claim>,
) -> Result<Entity, ParseError> {
    // Pull `with` / `maybe` out so we can resolve their values
    // (which may be bookmark refs, not full descriptors) before
    // handing the rest to dialog's deserializer.
    let with_block = fields.remove("with");
    let maybe_block = fields.remove("maybe");

    // Resolve each `with`/`maybe` entry into (field name,
    // attribute entity, optional inline-descriptor for hashing).
    let with_pairs = resolve_field_block(subject, with_block, bookmarks, out)?;
    let maybe_pairs = resolve_field_block(subject, maybe_block, bookmarks, out)?;

    // Build a ConceptDescriptor over the resolved attributes for
    // canonical entity derivation. We reconstruct AttributeDescriptors
    // from inline definitions where given; bookmark-only references
    // don't carry the descriptor and so are skipped from the hash
    // input. This is a v1 limitation: a concept defined entirely by
    // bookmark references would need a branch lookup to recover the
    // inline descriptors before hashing. Document-only inline
    // concepts work fully.
    let with_descriptors: Vec<(String, AttributeDescriptor)> = with_pairs
        .iter()
        .filter_map(|(name, _, desc)| desc.as_ref().map(|d| (name.clone(), d.clone())))
        .collect();
    let descriptor = build_concept_descriptor(&fields, with_descriptors, subject)?;
    let entity = descriptor.this();

    // Description.
    if let Some(desc) = descriptor.description()
        && !desc.is_empty()
    {
        out.push(Claim {
            the: meta_attr("description"),
            of: entity.clone(),
            is: Value::String(desc.to_owned()),
        });
    }

    // dialog.concept.with/{name} = attribute_entity per resolved
    // field (covers both inline-defined and bookmark-referenced
    // attributes — both contribute the link claim).
    for (name, attr_entity, _) in with_pairs {
        out.push(Claim {
            the: concept::with(&name).map_err(|e| ParseError::InvalidRelation {
                relation: format!("dialog.concept.with/{name}"),
                reason: e.to_string(),
            })?,
            of: entity.clone(),
            is: Value::Entity(attr_entity),
        });
    }

    for (name, attr_entity, _) in maybe_pairs {
        out.push(Claim {
            the: concept::maybe(&name).map_err(|e| ParseError::InvalidRelation {
                relation: format!("dialog.concept.maybe/{name}"),
                reason: e.to_string(),
            })?,
            of: entity.clone(),
            is: Value::Entity(attr_entity),
        });
    }

    Ok(entity)
}

/// Resolve one block (`with` or `maybe`) of a concept assertion.
///
/// Each entry is either:
/// - a string: bookmark name (look up in `bookmarks`) or URI literal
/// - an object: inline `attribute`-shaped descriptor (asserted in-place)
fn resolve_field_block(
    subject: &str,
    block: Option<Json>,
    bookmarks: &BTreeMap<String, Entity>,
    out: &mut Vec<Claim>,
) -> Result<Vec<(String, Entity, Option<AttributeDescriptor>)>, ParseError> {
    let Some(block) = block else {
        return Ok(Vec::new());
    };
    let Json::Object(map) = block else {
        return Err(ParseError::InvalidDescriptor {
            kind: "concept-fields",
            subject: subject.to_owned(),
            reason: "expected object of field-name -> reference-or-inline".to_owned(),
        });
    };

    let mut resolved = Vec::new();
    for (field_name, value) in map {
        match value {
            Json::String(reference) => {
                let entity = if reference.contains(':') {
                    reference
                        .parse()
                        .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                            ParseError::InvalidUri {
                                subject: reference.clone(),
                                reason: e.to_string(),
                            }
                        })?
                } else {
                    bookmarks
                        .get(&reference)
                        .cloned()
                        .ok_or(ParseError::UnknownBookmark {
                            subject: subject.to_owned(),
                            bookmark: reference.clone(),
                        })?
                };
                resolved.push((field_name, entity, None));
            }
            Json::Object(inline_fields) => {
                let descriptor: AttributeDescriptor =
                    serde_json::from_value(Json::Object(inline_fields.clone())).map_err(|e| {
                        ParseError::InvalidDescriptor {
                            kind: "attribute (inline)",
                            subject: subject.to_owned(),
                            reason: e.to_string(),
                        }
                    })?;
                let entity: Entity = descriptor
                    .to_uri()
                    .parse()
                    .expect("descriptor URI is always valid");
                push_descriptor_claims(&descriptor, &entity, out)?;
                resolved.push((field_name, entity, Some(descriptor)));
            }
            other => {
                return Err(ParseError::InvalidDescriptor {
                    kind: "concept-field",
                    subject: subject.to_owned(),
                    reason: format!(
                        "field {field_name:?} value must be a string (bookmark/URI) or object (inline attribute), got {}",
                        json_kind(&other)
                    ),
                });
            }
        }
    }
    Ok(resolved)
}

fn build_concept_descriptor(
    remaining_fields: &serde_json::Map<String, Json>,
    with_descriptors: Vec<(String, AttributeDescriptor)>,
    subject: &str,
) -> Result<ConceptDescriptor, ParseError> {
    // Compose a JSON shape dialog's deserializer accepts, then
    // overwrite `with` with the resolved descriptor list. This
    // round-trips description + any other dialog-recognized
    // top-level fields without us having to enumerate them.
    let mut shape = serde_json::Map::new();
    if let Some(desc) = remaining_fields.get("description") {
        shape.insert("description".to_owned(), desc.clone());
    }
    let with_obj: serde_json::Map<String, Json> = with_descriptors
        .into_iter()
        .map(|(name, descriptor)| {
            (
                name,
                serde_json::to_value(descriptor).expect("AttributeDescriptor is serializable"),
            )
        })
        .collect();
    shape.insert("with".to_owned(), Json::Object(with_obj));

    serde_json::from_value(Json::Object(shape)).map_err(|e| ParseError::InvalidDescriptor {
        kind: "concept",
        subject: subject.to_owned(),
        reason: e.to_string(),
    })
}

/// Emit the indexable facts for an attribute descriptor:
/// `dialog.attribute/{id,type,cardinality}` plus optional
/// `dialog.meta/description`.
fn push_descriptor_claims(
    descriptor: &AttributeDescriptor,
    entity: &Entity,
    out: &mut Vec<Claim>,
) -> Result<(), ParseError> {
    let selector = format!("{}/{}", descriptor.domain(), descriptor.name());
    out.push(Claim {
        the: attribute_attr("id"),
        of: entity.clone(),
        is: Value::String(selector),
    });

    if let Some(ty) = descriptor.content_type() {
        // Round-trip Type through serde to get the descriptor name
        // ("Text", "UnsignedInteger", …) rather than the underlying
        // ValueDataType variant ("String", "UnsignedInt", …).
        let type_name = serde_json::to_value(ty)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if !type_name.is_empty() {
            out.push(Claim {
                the: attribute_attr("type"),
                of: entity.clone(),
                is: Value::String(type_name),
            });
        }
    }

    let cardinality_name = serde_json::to_value(descriptor.cardinality())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "one".to_owned());
    out.push(Claim {
        the: attribute_attr("cardinality"),
        of: entity.clone(),
        is: Value::String(cardinality_name),
    });

    let description = descriptor.description();
    if !description.is_empty() {
        out.push(Claim {
            the: meta_attr("description"),
            of: entity.clone(),
            is: Value::String(description.to_owned()),
        });
    }

    Ok(())
}

fn emit_bookmark_name(name: &str, entity: &Entity, out: &mut Vec<Claim>) -> Result<(), ParseError> {
    out.push(Claim {
        the: meta_attr("name"),
        of: entity.clone(),
        is: Value::String(name.to_owned()),
    });
    Ok(())
}

fn meta_attr(name: &str) -> ArtifactsAttribute {
    format!("dialog.meta/{name}")
        .parse()
        .expect("dialog.meta/<name> is always a valid relation")
}

fn attribute_attr(name: &str) -> ArtifactsAttribute {
    format!("dialog.attribute/{name}")
        .parse()
        .expect("dialog.attribute/<name> is always a valid relation")
}

fn make_relation(domain: &str, field: &str) -> Result<ArtifactsAttribute, ParseError> {
    format!("{domain}/{field}")
        .parse()
        .map_err(
            |e: dialog_artifacts::DialogArtifactsError| ParseError::InvalidRelation {
                relation: format!("{domain}/{field}"),
                reason: e.to_string(),
            },
        )
}

fn json_scalar_to_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::String(String::new()),
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Value::UnsignedInt(i as u128)
                } else {
                    Value::SignedInt(i as i128)
                }
            } else if let Some(u) = n.as_u64() {
                Value::UnsignedInt(u as u128)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::String(n.to_string())
            }
        }
        Json::String(s) => Value::String(s.clone()),
        // Caller guards against object/array via NestedNotSupported.
        other => Value::String(other.to_string()),
    }
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

fn yaml_to_json(value: serde_yaml::Value) -> Result<Json, String> {
    match value {
        serde_yaml::Value::Null => Ok(Json::Null),
        serde_yaml::Value::Bool(b) => Ok(Json::Bool(b)),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Json::Number(i.into()))
            } else if let Some(u) = n.as_u64() {
                Ok(Json::Number(u.into()))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(Json::Number)
                    .ok_or_else(|| format!("non-finite float: {f}"))
            } else {
                Err(format!("unrepresentable number: {n:?}"))
            }
        }
        serde_yaml::Value::String(s) => Ok(Json::String(s)),
        serde_yaml::Value::Sequence(seq) => seq
            .into_iter()
            .map(yaml_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Json::Array),
        serde_yaml::Value::Mapping(map) => {
            let mut obj = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    serde_yaml::Value::Number(n) => n.to_string(),
                    other => return Err(format!("non-stringable map key: {other:?}")),
                };
                obj.insert(key, yaml_to_json(v)?);
            }
            Ok(Json::Object(obj))
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relation(claim: &Claim) -> String {
        String::from(&claim.the)
    }

    fn value_string(claim: &Claim) -> &str {
        match &claim.is {
            Value::String(s) => s.as_str(),
            other => panic!("expected string value, got {other:?}"),
        }
    }

    #[test]
    fn empty_document_is_noop() {
        let tx = parse_json("{}").unwrap();
        assert!(tx.claims.is_empty());
        assert!(tx.bookmarks.is_empty());
    }

    #[test]
    fn anonymous_subject_is_rejected() {
        let err = parse_json(r#"{ "_": { "xyz.tonk.demo": { "foo": 1 } } }"#).unwrap_err();
        assert!(matches!(err, ParseError::AnonymousSubjectNotSupported));
    }

    #[test]
    fn bookmark_subject_in_domain_context_resolves_deterministically() {
        let tx1 =
            parse_json(r#"{ "alice": { "com.app.person": { "name": "Alice", "age": 26 } } }"#)
                .unwrap();
        let tx2 =
            parse_json(r#"{ "alice": { "com.app.person": { "name": "Alice", "age": 27 } } }"#)
                .unwrap();
        // Same bookmark name → same entity, regardless of field
        // content. Updates land on the same entity.
        assert_eq!(tx1.bookmarks["alice"], tx2.bookmarks["alice"]);
        assert!(tx1.bookmarks["alice"].to_string().starts_with("did:key:z"));
    }

    #[test]
    fn attribute_definition_emits_index_facts() {
        let tx = parse_json(
            r#"{
                "person-name": {
                    "attribute": {
                        "description": "The person's name",
                        "the": "io.gozala.person/name",
                        "as": "Text",
                        "cardinality": "one"
                    }
                }
            }"#,
        )
        .unwrap();

        let entity = tx.bookmarks.get("person-name").expect("bookmark recorded");
        assert!(entity.to_string().starts_with("the:"));

        // Expect 5 claims: id, type, cardinality, description, name (bookmark).
        assert_eq!(tx.claims.len(), 5, "claims: {:#?}", tx.claims);
        for c in &tx.claims {
            assert_eq!(&c.of, entity);
        }

        let by_relation: BTreeMap<String, &Claim> =
            tx.claims.iter().map(|c| (relation(c), c)).collect();

        assert_eq!(
            value_string(by_relation["dialog.attribute/id"]),
            "io.gozala.person/name"
        );
        assert_eq!(value_string(by_relation["dialog.attribute/type"]), "Text");
        assert_eq!(
            value_string(by_relation["dialog.attribute/cardinality"]),
            "one"
        );
        assert_eq!(
            value_string(by_relation["dialog.meta/description"]),
            "The person's name"
        );
        assert_eq!(value_string(by_relation["dialog.meta/name"]), "person-name");
    }

    #[test]
    fn concept_with_bookmark_references_resolves() {
        let tx = parse_json(
            r#"{
                "person-name": {
                    "attribute": {
                        "the": "io.gozala.person/name",
                        "as": "Text",
                        "cardinality": "one"
                    }
                },
                "person-age": {
                    "attribute": {
                        "the": "io.gozala.person/age",
                        "as": "UnsignedInteger",
                        "cardinality": "one"
                    }
                },
                "person": {
                    "concept": {
                        "description": "A person",
                        "with": {
                            "name": "person-name",
                            "age":  "person-age"
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let person = tx.bookmarks.get("person").expect("person bookmark");
        let name_attr = tx.bookmarks.get("person-name").unwrap().clone();
        let age_attr = tx.bookmarks.get("person-age").unwrap().clone();

        // The concept entity itself should be the result of dialog's
        // structural hash.  A bookmark-reference-only concept has no
        // inline descriptors to feed the hash, so v1 produces a
        // descriptor with empty `with` — the concept entity is
        // therefore the canonical "empty concept". This is a v1
        // limitation noted in `emit_concept`.
        assert!(person.to_string().starts_with("concept:"));

        // Find the with/{name} and with/{age} link claims pointing
        // to the right attribute entities.
        let with_name: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.concept.with/name" && c.of == *person)
            .collect();
        assert_eq!(with_name.len(), 1);
        assert_eq!(with_name[0].is, Value::Entity(name_attr));

        let with_age: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.concept.with/age" && c.of == *person)
            .collect();
        assert_eq!(with_age.len(), 1);
        assert_eq!(with_age[0].is, Value::Entity(age_attr));
    }

    #[test]
    fn concept_with_inline_attributes_full_round_trip() {
        let tx = parse_json(
            r#"{
                "person": {
                    "concept": {
                        "description": "A person",
                        "with": {
                            "name": {
                                "the": "io.gozala.person/name",
                                "as": "Text",
                                "cardinality": "one"
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let person = tx.bookmarks.get("person").expect("person bookmark");
        // With inline descriptor present, the concept's identity
        // is over the resolved attribute set — not the empty hash.
        assert!(person.to_string().starts_with("concept:"));

        // Inline attribute should also have produced its own
        // index facts (id/type/cardinality on the attribute entity).
        let attr_id_claims: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.attribute/id")
            .collect();
        assert_eq!(attr_id_claims.len(), 1);
        assert_eq!(value_string(attr_id_claims[0]), "io.gozala.person/name");
    }

    #[test]
    fn yaml_round_trips_through_json() {
        let yaml = r#"
person-name:
  attribute:
    description: The person's name
    the: io.gozala.person/name
    as: Text
    cardinality: one
"#;
        let tx = parse_yaml(yaml).unwrap();
        assert_eq!(tx.bookmarks.len(), 1);
        assert!(tx.bookmarks.contains_key("person-name"));
    }

    #[test]
    fn unknown_concept_context_errors() {
        let err = parse_json(r#"{ "x": { "person": { "name": "Alice" } } }"#).unwrap_err();
        match err {
            ParseError::UnsupportedContext { context, .. } => assert_eq!(context, "person"),
            other => panic!("expected UnsupportedContext, got {other:?}"),
        }
    }

    #[test]
    fn bookmark_reference_to_undefined_errors() {
        let err =
            parse_json(r#"{ "p": { "concept": { "with": { "name": "missing" } } } }"#).unwrap_err();
        match err {
            ParseError::UnknownBookmark { bookmark, .. } => assert_eq!(bookmark, "missing"),
            other => panic!("expected UnknownBookmark, got {other:?}"),
        }
    }

    #[test]
    fn nested_object_in_domain_context_errors() {
        let err = parse_json(r#"{ "alice": { "xyz.tonk": { "nested": {"a": 1} } } }"#).unwrap_err();
        match err {
            ParseError::NestedNotSupported { field, .. } => assert_eq!(field, "nested"),
            other => panic!("expected NestedNotSupported, got {other:?}"),
        }
    }
}
