//! Interpreter — turns a [`tonk_notation::Syntax`] tree into a
//! [`Transaction`] of EAV claims ready to commit to a branch.
//!
//! The interpreter resolves bookmark references (document-then-
//! branch via a [`Resolver`]), reconstructs canonical entity URIs
//! via dialog's `AttributeDescriptor::this()` /
//! `ConceptDescriptor::this()`, and emits the indexable facts
//! (`dialog.attribute/{id,type,cardinality}`,
//! `dialog.concept.with/{name}`, `dialog.meta/{name,description}`,
//! plus the raw EAV writes under bare-domain contexts).
//!
//! The split from the parser is deliberate: the parser knows YAML/
//! JSON shape; the interpreter knows dialog's meta-schema. The
//! same architecture is what an LSP completion implementation
//! would use to query "which attribute does the bookmark
//! `person-name` point at" from inside the editor.

use std::collections::BTreeMap;

use async_trait::async_trait;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_common::Blake3Hash;
use dialog_query::{AttributeDescriptor, ConceptDescriptor};
use thiserror::Error;
use tonk_notation::{
    AttributeNode, ConceptField as SyntaxConceptField, ConceptNode, Context, DomainContext,
    DomainField, DomainValue, Reference, Scalar as SyntaxScalar, Statement, Subject, SubjectKind,
    Syntax,
};

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

/// Result of interpreting a [`Syntax`] tree.
#[derive(Debug, Default)]
pub struct Transaction {
    /// EAV claims to be asserted in commit order.
    pub claims: Vec<Claim>,
    /// Bookmark name → resolved entity, for each named subject in
    /// the document.
    pub bookmarks: BTreeMap<String, Entity>,
}

/// Errors raised while interpreting a [`Syntax`] tree.
#[derive(Debug, Error)]
pub enum InterpretError {
    /// `_:` (anonymous) subjects aren't supported. Use a bookmark
    /// name or a URI literal.
    #[error("`_` (anonymous) subject is not supported; use a bookmark name or a URI")]
    AnonymousSubject,
    /// `?var:` subjects aren't supported outside of query / rule
    /// contexts (which the interpreter doesn't yet know about).
    #[error("`?{name}` (variable) subject is not supported in transaction context")]
    VariableSubject {
        /// The variable name (without the `?` prefix).
        name: String,
    },
    /// A subject that looked like a URI didn't parse as one.
    #[error("subject {subject:?} is not a valid entity URI: {reason}")]
    InvalidUri {
        /// The offending subject text.
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
    /// An `attribute:` block was missing required fields or had
    /// invalid values.
    #[error("invalid attribute on subject {subject:?}: {reason}")]
    InvalidAttribute {
        /// The subject the attribute was under.
        subject: String,
        /// Description of the problem.
        reason: String,
    },
    /// A `concept:` block was missing required fields or had
    /// invalid values.
    #[error("invalid concept on subject {subject:?}: {reason}")]
    InvalidConcept {
        /// The subject the concept was under.
        subject: String,
        /// Description of the problem.
        reason: String,
    },
    /// A bookmark reference inside a concept's `with`/`maybe`
    /// could not be resolved against the document or the branch.
    #[error("concept on subject {subject:?} references unknown bookmark {bookmark:?}")]
    UnknownBookmark {
        /// The subject containing the reference.
        subject: String,
        /// The unresolved bookmark name.
        bookmark: String,
    },
    /// A reference resolved to an entity that isn't an attribute
    /// (no descriptor recoverable from the branch's
    /// `dialog.attribute/*` claims).
    #[error("reference {reference:?} on subject {subject:?} is not an attribute")]
    NotAnAttribute {
        /// The subject containing the reference.
        subject: String,
        /// The bookmark name or URI that resolved to a
        /// non-attribute.
        reference: String,
    },
    /// User-defined concept contexts aren't yet supported by the
    /// interpreter — they require a branch-side schema lookup.
    #[error("user-defined concept context {context:?} on subject {subject:?} is not yet supported")]
    UserConceptUnsupported {
        /// The subject the context appeared on.
        subject: String,
        /// The unrecognised context key.
        context: String,
    },
    /// A nested map appeared in a domain-context field; nested
    /// entities under domain context aren't yet supported.
    #[error("nested object under {context:?}.{field:?} is not yet supported")]
    NestedNotSupported {
        /// The domain.
        context: String,
        /// The field with the nested map.
        field: String,
    },
    /// The resolver's underlying I/O failed.
    #[error("resolver error for {bookmark:?}: {reason}")]
    ResolverFailed {
        /// The bookmark that failed to resolve.
        bookmark: String,
        /// Underlying message.
        reason: String,
    },
}

/// An attribute resolved from outside the current document — its
/// entity URI plus the descriptor that produced it.
///
/// Both pieces are required so the interpreter can compute the
/// canonical concept entity (which hashes over the full
/// descriptor map, not just the URIs).
#[derive(Debug, Clone)]
pub struct ResolvedAttribute {
    /// The attribute's entity URI (`the:…` shape from
    /// [`AttributeDescriptor::to_uri`]).
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: AttributeDescriptor,
}

/// Look up a bookmark name against a backing store.
///
/// The interpreter calls this when it encounters a bookmark
/// reference that wasn't defined in the current document.
/// Implementations typically query the branch for an entity
/// carrying `dialog.meta/name = <name>` and reconstruct the
/// descriptor from its `dialog.attribute/*` facts.
///
/// The trait is async; pass a real branch-backed implementation
/// when interpreting a transaction body, or use [`NoopResolver`]
/// for unit tests that don't exercise cross-document refs.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Resolver {
    /// Resolve a bookmark name to a [`ResolvedAttribute`], or
    /// `Ok(None)` when the name isn't known. Errors should be
    /// reserved for genuine I/O failures, not "not found".
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError>;

    /// Resolve an attribute entity URI to a [`ResolvedAttribute`].
    ///
    /// Used when a concept's `with` / `maybe` block references an
    /// attribute by its `the:…` URI rather than by bookmark
    /// name. The resolver looks up the
    /// `dialog.attribute/{id,type,cardinality}` plus
    /// `dialog.meta/description` facts on the supplied entity
    /// and reconstructs the
    /// [`AttributeDescriptor`][dialog_query::AttributeDescriptor]
    /// — without it, the interpreter can't compute the concept's
    /// canonical hash for a URI-only reference.
    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError>;
}

/// Opaque error from a [`Resolver`] implementation. The
/// interpreter wraps this into [`InterpretError::ResolverFailed`]
/// without inspecting it.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ResolverError {
    /// Human-readable description of the underlying failure.
    pub message: String,
}

impl ResolverError {
    /// Create a new resolver error with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A [`Resolver`] that always returns `None`. Convenient for
/// document-only interpretation paths (tests, single-shot
/// transactions where the caller knows everything is in the
/// document) and as the base case in the route handler before
/// the branch resolver is wired in.
pub struct NoopResolver;

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Resolver for NoopResolver {
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
}

/// Interpret a [`Syntax`] tree into a [`Transaction`].
///
/// The `resolver` is consulted for bookmark references that
/// aren't satisfied from the document's own bookmark map.
pub async fn interpret<R: Resolver>(
    syntax: &Syntax,
    resolver: &R,
) -> Result<Transaction, InterpretError> {
    let mut tx = Transaction::default();
    // Track attribute bookmarks within the document with their
    // descriptors so concept hashing can use the canonical
    // identity even when fields reference attributes by name.
    let mut local_attributes: BTreeMap<String, ResolvedAttribute> = BTreeMap::new();

    for statement in &syntax.statements {
        interpret_statement(statement, &mut tx, &mut local_attributes, resolver).await?;
    }
    Ok(tx)
}

async fn interpret_statement<R: Resolver>(
    statement: &Statement,
    tx: &mut Transaction,
    local_attributes: &mut BTreeMap<String, ResolvedAttribute>,
    resolver: &R,
) -> Result<(), InterpretError> {
    let subject_text = statement.subject.source.clone();
    let mut entity: Option<Entity> = None;

    for context in &statement.contexts {
        let resolved = match context {
            Context::Domain(domain) => {
                let e = materialize_subject(&statement.subject, entity.as_ref())?;
                emit_domain(domain, &e, &mut tx.claims)?;
                e
            }
            Context::Attribute(attr) => {
                let resolved = build_attribute(&subject_text, attr)?;
                push_attribute_claims(&resolved, &mut tx.claims)?;
                if let SubjectKind::Bookmark = statement.subject.kind {
                    local_attributes.insert(subject_text.clone(), resolved.clone());
                }
                resolved.entity
            }
            Context::Concept(concept) => {
                build_and_emit_concept(
                    &subject_text,
                    concept,
                    &tx.bookmarks,
                    local_attributes,
                    resolver,
                    &mut tx.claims,
                )
                .await?
            }
            Context::UserConcept(user) => {
                return Err(InterpretError::UserConceptUnsupported {
                    subject: subject_text,
                    context: user.name.clone(),
                });
            }
        };
        entity = Some(resolved);
    }

    if let SubjectKind::Bookmark = statement.subject.kind
        && let Some(e) = entity.clone()
    {
        push_bookmark_name_claim(&subject_text, &e, &mut tx.claims);
        tx.bookmarks.insert(subject_text, e);
    }

    Ok(())
}

fn materialize_subject(
    subject: &Subject,
    existing: Option<&Entity>,
) -> Result<Entity, InterpretError> {
    if let Some(e) = existing {
        return Ok(e.clone());
    }
    match subject.kind {
        SubjectKind::Anonymous => Err(InterpretError::AnonymousSubject),
        SubjectKind::Variable => Err(InterpretError::VariableSubject {
            name: subject.source.trim_start_matches('?').to_owned(),
        }),
        SubjectKind::Uri => {
            subject
                .source
                .parse()
                .map_err(
                    |e: dialog_artifacts::DialogArtifactsError| InterpretError::InvalidUri {
                        subject: subject.source.clone(),
                        reason: e.to_string(),
                    },
                )
        }
        SubjectKind::Bookmark => Ok(derive_entity(&subject.source)),
    }
}

fn emit_domain(
    domain: &DomainContext,
    entity: &Entity,
    out: &mut Vec<Claim>,
) -> Result<(), InterpretError> {
    for field in &domain.fields {
        emit_domain_field(&domain.domain, field, entity, out)?;
    }
    Ok(())
}

fn emit_domain_field(
    domain: &str,
    field: &DomainField,
    entity: &Entity,
    out: &mut Vec<Claim>,
) -> Result<(), InterpretError> {
    let relation = make_relation(domain, &field.name)?;
    match &field.value {
        DomainValue::Scalar(scalar) => {
            out.push(Claim {
                the: relation,
                of: entity.clone(),
                is: scalar_to_value(scalar),
            });
        }
        DomainValue::Sequence(items) => {
            for item in items {
                let DomainValue::Scalar(scalar) = item else {
                    return Err(InterpretError::NestedNotSupported {
                        context: domain.to_owned(),
                        field: field.name.clone(),
                    });
                };
                out.push(Claim {
                    the: relation.clone(),
                    of: entity.clone(),
                    is: scalar_to_value(scalar),
                });
            }
        }
        DomainValue::Mapping(_) => {
            return Err(InterpretError::NestedNotSupported {
                context: domain.to_owned(),
                field: field.name.clone(),
            });
        }
    }
    Ok(())
}

fn build_attribute(
    subject: &str,
    node: &AttributeNode,
) -> Result<ResolvedAttribute, InterpretError> {
    // Use serde to round-trip through dialog's own deserializer so
    // we don't have to mirror its `as` ↔ Type mapping or its
    // cardinality string handling.
    let mut shape = serde_json::Map::new();
    let the = node
        .the
        .as_ref()
        .ok_or_else(|| InterpretError::InvalidAttribute {
            subject: subject.to_owned(),
            reason: "missing required field `the`".to_owned(),
        })?;
    shape.insert(
        "the".to_owned(),
        serde_json::Value::String(the.value.clone()),
    );
    if let Some(s) = &node.as_type {
        shape.insert("as".to_owned(), serde_json::Value::String(s.value.clone()));
    }
    if let Some(s) = &node.cardinality {
        shape.insert(
            "cardinality".to_owned(),
            serde_json::Value::String(s.value.clone()),
        );
    }
    if let Some(s) = &node.description {
        shape.insert(
            "description".to_owned(),
            serde_json::Value::String(s.value.clone()),
        );
    }
    let descriptor: AttributeDescriptor = serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| InterpretError::InvalidAttribute {
            subject: subject.to_owned(),
            reason: e.to_string(),
        })?;
    let entity: Entity = descriptor
        .to_uri()
        .parse()
        .expect("AttributeDescriptor::to_uri produces a valid entity URI");
    Ok(ResolvedAttribute { entity, descriptor })
}

fn push_attribute_claims(
    resolved: &ResolvedAttribute,
    out: &mut Vec<Claim>,
) -> Result<(), InterpretError> {
    let descriptor = &resolved.descriptor;
    let selector = format!("{}/{}", descriptor.domain(), descriptor.name());
    out.push(Claim {
        the: attribute_attr("id"),
        of: resolved.entity.clone(),
        is: Value::String(selector),
    });
    if let Some(ty) = descriptor.content_type() {
        let type_name = serde_json::to_value(ty)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if !type_name.is_empty() {
            out.push(Claim {
                the: attribute_attr("type"),
                of: resolved.entity.clone(),
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
        of: resolved.entity.clone(),
        is: Value::String(cardinality_name),
    });
    let description = descriptor.description();
    if !description.is_empty() {
        out.push(Claim {
            the: meta_attr("description"),
            of: resolved.entity.clone(),
            is: Value::String(description.to_owned()),
        });
    }
    Ok(())
}

async fn build_and_emit_concept<R: Resolver>(
    subject: &str,
    node: &ConceptNode,
    document_bookmarks: &BTreeMap<String, Entity>,
    local_attributes: &mut BTreeMap<String, ResolvedAttribute>,
    resolver: &R,
    out: &mut Vec<Claim>,
) -> Result<Entity, InterpretError> {
    let with = resolve_concept_fields(
        subject,
        &node.with,
        document_bookmarks,
        local_attributes,
        resolver,
        out,
    )
    .await?;
    let maybe = resolve_concept_fields(
        subject,
        &node.maybe,
        document_bookmarks,
        local_attributes,
        resolver,
        out,
    )
    .await?;

    let mut shape = serde_json::Map::new();
    if let Some(desc) = &node.description {
        shape.insert(
            "description".to_owned(),
            serde_json::Value::String(desc.value.clone()),
        );
    }
    let with_obj: serde_json::Map<String, serde_json::Value> = with
        .iter()
        .map(|(name, resolved)| {
            (
                name.clone(),
                serde_json::to_value(&resolved.descriptor)
                    .expect("AttributeDescriptor is serializable"),
            )
        })
        .collect();
    shape.insert("with".to_owned(), serde_json::Value::Object(with_obj));
    let descriptor: ConceptDescriptor = serde_json::from_value(serde_json::Value::Object(shape))
        .map_err(|e| InterpretError::InvalidConcept {
            subject: subject.to_owned(),
            reason: e.to_string(),
        })?;
    let entity = descriptor.this();

    if let Some(desc) = descriptor.description()
        && !desc.is_empty()
    {
        out.push(Claim {
            the: meta_attr("description"),
            of: entity.clone(),
            is: Value::String(desc.to_owned()),
        });
    }
    for (name, resolved) in with {
        let relation = concept::with(&name).map_err(|e| InterpretError::InvalidRelation {
            relation: format!("dialog.concept.with/{name}"),
            reason: e.to_string(),
        })?;
        out.push(Claim {
            the: relation,
            of: entity.clone(),
            is: Value::Entity(resolved.entity),
        });
    }
    for (name, resolved) in maybe {
        let relation = concept::maybe(&name).map_err(|e| InterpretError::InvalidRelation {
            relation: format!("dialog.concept.maybe/{name}"),
            reason: e.to_string(),
        })?;
        out.push(Claim {
            the: relation,
            of: entity.clone(),
            is: Value::Entity(resolved.entity),
        });
    }
    Ok(entity)
}

async fn resolve_concept_fields<R: Resolver>(
    subject: &str,
    fields: &[SyntaxConceptField],
    document_bookmarks: &BTreeMap<String, Entity>,
    local_attributes: &mut BTreeMap<String, ResolvedAttribute>,
    resolver: &R,
    out: &mut Vec<Claim>,
) -> Result<Vec<(String, ResolvedAttribute)>, InterpretError> {
    let mut resolved_fields = Vec::with_capacity(fields.len());
    for field in fields {
        let resolved = resolve_reference(
            subject,
            &field.value,
            document_bookmarks,
            local_attributes,
            resolver,
            out,
        )
        .await?;
        resolved_fields.push((field.name.clone(), resolved));
    }
    Ok(resolved_fields)
}

async fn resolve_reference<R: Resolver>(
    subject: &str,
    reference: &Reference,
    document_bookmarks: &BTreeMap<String, Entity>,
    local_attributes: &mut BTreeMap<String, ResolvedAttribute>,
    resolver: &R,
    out: &mut Vec<Claim>,
) -> Result<ResolvedAttribute, InterpretError> {
    match reference {
        Reference::Inline(node) => {
            let resolved = build_attribute(subject, node)?;
            push_attribute_claims(&resolved, out)?;
            Ok(resolved)
        }
        Reference::Bookmark(spanned) => {
            let name = &spanned.value;
            if let Some(resolved) = local_attributes.get(name) {
                return Ok(resolved.clone());
            }
            // Possibly a non-attribute bookmark (e.g. an entity
            // bound under a domain context). The current model
            // requires concept fields to be attributes, so anything
            // non-attribute is an error.
            if document_bookmarks.contains_key(name) {
                return Err(InterpretError::NotAnAttribute {
                    subject: subject.to_owned(),
                    reference: name.clone(),
                });
            }
            match resolver.resolve_attribute(name).await.map_err(|e| {
                InterpretError::ResolverFailed {
                    bookmark: name.clone(),
                    reason: e.message,
                }
            })? {
                Some(resolved) => {
                    local_attributes.insert(name.clone(), resolved.clone());
                    Ok(resolved)
                }
                None => Err(InterpretError::UnknownBookmark {
                    subject: subject.to_owned(),
                    bookmark: name.clone(),
                }),
            }
        }
        Reference::Uri(spanned) => {
            // Parse the URI as an entity — anything that fails
            // here surfaces as InvalidUri so the user gets a
            // clearer message than the generic "not an attribute".
            let entity: Entity =
                spanned
                    .value
                    .parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        InterpretError::InvalidUri {
                            subject: spanned.value.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            match resolver
                .resolve_attribute_by_entity(&entity)
                .await
                .map_err(|e| InterpretError::ResolverFailed {
                    bookmark: spanned.value.clone(),
                    reason: e.message,
                })? {
                Some(resolved) => Ok(resolved),
                None => Err(InterpretError::NotAnAttribute {
                    subject: subject.to_owned(),
                    reference: spanned.value.clone(),
                }),
            }
        }
    }
}

fn push_bookmark_name_claim(name: &str, entity: &Entity, out: &mut Vec<Claim>) {
    out.push(Claim {
        the: meta_attr("name"),
        of: entity.clone(),
        is: Value::String(name.to_owned()),
    });
}

// -----------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------

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

fn make_relation(domain: &str, field: &str) -> Result<ArtifactsAttribute, InterpretError> {
    format!("{domain}/{field}")
        .parse()
        .map_err(
            |e: dialog_artifacts::DialogArtifactsError| InterpretError::InvalidRelation {
                relation: format!("{domain}/{field}"),
                reason: e.to_string(),
            },
        )
}

fn scalar_to_value(scalar: &SyntaxScalar) -> Value {
    match scalar {
        SyntaxScalar::Null => Value::String(String::new()),
        SyntaxScalar::Boolean(b) => Value::Boolean(*b),
        SyntaxScalar::Integer(i) => {
            if *i >= 0 {
                Value::UnsignedInt(*i as u128)
            } else {
                Value::SignedInt(*i)
            }
        }
        SyntaxScalar::UnsignedInteger(u) => Value::UnsignedInt(*u),
        SyntaxScalar::Float(f) => Value::Float(*f),
        SyntaxScalar::String(s) => Value::String(s.clone()),
    }
}

/// Resolve a bookmark name to its `did:key:z…` entity URI.
///
/// We hash the name with blake3 and encode the result directly
/// as the multicodec payload (skipping carry's
/// ed25519-keypair-derivation step so the parser can stay sync).
/// The same caveat as the previous transact module applies:
/// domain-context bookmark entities don't byte-match carry.
fn derive_entity(name: &str) -> Entity {
    let hash = Blake3Hash::hash(name.as_bytes());
    const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];
    let mut payload = [0u8; 34];
    payload[..2].copy_from_slice(&ED25519_MULTICODEC);
    payload[2..].copy_from_slice(hash.as_bytes());
    let encoded = bs58::encode(&payload).into_string();
    format!("did:key:z{encoded}")
        .parse()
        .expect("did:key URI built from a 34-byte multicodec payload is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonk_notation::parse_json;

    /// Stub [`Resolver`] backed by an in-memory map. Mirrors what
    /// the worker's `BranchResolver` does at the top of the call
    /// graph, without needing a real branch — the only realism
    /// that matters at this layer is "name → descriptor" or
    /// "entity → descriptor".
    struct StubResolver {
        by_name: BTreeMap<String, ResolvedAttribute>,
        by_entity: BTreeMap<Entity, ResolvedAttribute>,
    }

    impl StubResolver {
        fn empty() -> Self {
            Self {
                by_name: BTreeMap::new(),
                by_entity: BTreeMap::new(),
            }
        }

        fn with_named(mut self, name: &str, attr: ResolvedAttribute) -> Self {
            self.by_name.insert(name.to_owned(), attr);
            self
        }

        fn with_uri(mut self, attr: ResolvedAttribute) -> Self {
            self.by_entity.insert(attr.entity.clone(), attr);
            self
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Resolver for StubResolver {
        async fn resolve_attribute(
            &self,
            name: &str,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(self.by_name.get(name).cloned())
        }

        async fn resolve_attribute_by_entity(
            &self,
            entity: &Entity,
        ) -> Result<Option<ResolvedAttribute>, ResolverError> {
            Ok(self.by_entity.get(entity).cloned())
        }
    }

    /// Convenience: synthesize a `ResolvedAttribute` from a
    /// `the/as/cardinality` triple — the same way the
    /// interpreter does it on the write path.
    fn fake_attribute(the: &str, as_type: &str, cardinality: &str) -> ResolvedAttribute {
        let descriptor: AttributeDescriptor = serde_json::from_value(serde_json::json!({
            "the": the,
            "as": as_type,
            "cardinality": cardinality
        }))
        .unwrap();
        let entity: Entity = descriptor.to_uri().parse().unwrap();
        ResolvedAttribute { entity, descriptor }
    }

    fn relation(claim: &Claim) -> String {
        String::from(&claim.the)
    }

    fn value_string(claim: &Claim) -> &str {
        match &claim.is {
            Value::String(s) => s.as_str(),
            other => panic!("expected string value, got {other:?}"),
        }
    }

    /// Parse a JSON snippet into a `Syntax`, panicking on any
    /// parse diagnostic — keeps each test a single-line setup.
    fn parse(json: &str) -> Syntax {
        let parsed = parse_json(json);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        parsed.syntax.expect("syntax should be Some")
    }

    #[dialog_common::test]
    async fn empty_document_is_noop() {
        let syntax = parse("{}");
        let tx = interpret(&syntax, &NoopResolver).await.unwrap();
        assert!(tx.claims.is_empty());
        assert!(tx.bookmarks.is_empty());
    }

    #[dialog_common::test]
    async fn attribute_emits_index_facts() {
        let syntax = parse(
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
        );
        let tx = interpret(&syntax, &NoopResolver).await.unwrap();
        let entity = tx.bookmarks.get("person-name").expect("bookmark recorded");
        assert!(entity.to_string().starts_with("the:"));
        assert_eq!(tx.claims.len(), 5);
        let by_relation: BTreeMap<_, _> = tx.claims.iter().map(|c| (relation(c), c)).collect();
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

    /// Form 1: inline attribute descriptor as a concept's `with`
    /// field value.
    #[dialog_common::test]
    async fn concept_field_inline_descriptor() {
        let syntax = parse(
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
        );
        let tx = interpret(&syntax, &NoopResolver).await.unwrap();
        let entity = tx.bookmarks.get("person").expect("person bookmark");
        assert!(entity.to_string().starts_with("concept:"));
    }

    /// Form 2: bookmark reference to a sibling `attribute:`
    /// statement in the same document.
    #[dialog_common::test]
    async fn concept_field_local_bookmark() {
        let syntax = parse(
            r#"{
                "person-name": {
                    "attribute": {
                        "the": "io.gozala.person/name",
                        "as": "Text",
                        "cardinality": "one"
                    }
                },
                "person": {
                    "concept": {
                        "with": {
                            "name": "person-name"
                        }
                    }
                }
            }"#,
        );
        let tx = interpret(&syntax, &NoopResolver).await.unwrap();
        let person = tx.bookmarks.get("person").expect("person bookmark");
        assert!(person.to_string().starts_with("concept:"));
        let with_name: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.concept.with/name" && c.of == *person)
            .collect();
        assert_eq!(with_name.len(), 1);
    }

    /// Form 3: bookmark reference resolved via the resolver
    /// (cross-document — the user's reported bug).
    #[dialog_common::test]
    async fn concept_field_remote_bookmark() {
        let attr = fake_attribute("io.gozala.person/name", "Text", "one");
        let resolver = StubResolver::empty().with_named("person-name", attr.clone());

        let syntax = parse(
            r#"{
                "person": {
                    "concept": {
                        "with": { "name": "person-name" }
                    }
                }
            }"#,
        );
        let tx = interpret(&syntax, &resolver).await.unwrap();
        let person = tx.bookmarks.get("person").unwrap();
        let with_name: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.concept.with/name" && c.of == *person)
            .collect();
        assert_eq!(with_name.len(), 1);
        assert_eq!(with_name[0].is, Value::Entity(attr.entity));
    }

    /// Form 4: URI literal in the value position — resolved via
    /// `resolve_attribute_by_entity`.
    #[dialog_common::test]
    async fn concept_field_uri_literal() {
        let attr = fake_attribute("io.gozala.person/name", "Text", "one");
        let uri = attr.entity.to_string();
        let resolver = StubResolver::empty().with_uri(attr.clone());

        let json = format!(
            r#"{{
                "person": {{
                    "concept": {{
                        "with": {{ "name": "{uri}" }}
                    }}
                }}
            }}"#
        );
        let syntax = parse(&json);
        let tx = interpret(&syntax, &resolver).await.unwrap();
        let person = tx.bookmarks.get("person").unwrap();
        let with_name: Vec<&Claim> = tx
            .claims
            .iter()
            .filter(|c| relation(c) == "dialog.concept.with/name" && c.of == *person)
            .collect();
        assert_eq!(with_name.len(), 1);
        assert_eq!(with_name[0].is, Value::Entity(attr.entity));
    }

    /// URI references that don't resolve through the resolver
    /// surface as `NotAnAttribute`. Distinct from
    /// `UnknownBookmark` so users can tell the failure mode at
    /// a glance.
    #[dialog_common::test]
    async fn concept_field_uri_unknown_to_resolver() {
        // Build a syntactically-valid `the:…` URI that isn't
        // wired into the resolver.
        let dangling = fake_attribute("io.gozala.person/dangling", "Text", "one");
        let json = format!(
            r#"{{
                "person": {{
                    "concept": {{
                        "with": {{ "name": "{}" }}
                    }}
                }}
            }}"#,
            dangling.entity
        );
        let syntax = parse(&json);
        let err = interpret(&syntax, &NoopResolver).await.unwrap_err();
        match err {
            InterpretError::NotAnAttribute { reference, .. } => {
                assert_eq!(reference, dangling.entity.to_string());
            }
            other => panic!("expected NotAnAttribute, got {other:?}"),
        }
    }

    #[dialog_common::test]
    async fn concept_with_unknown_bookmark_errors() {
        let syntax = parse(r#"{ "p": { "concept": { "with": { "name": "missing" } } } }"#);
        let err = interpret(&syntax, &NoopResolver).await.unwrap_err();
        match err {
            InterpretError::UnknownBookmark { bookmark, .. } => {
                assert_eq!(bookmark, "missing");
            }
            other => panic!("expected UnknownBookmark, got {other:?}"),
        }
    }

    #[dialog_common::test]
    async fn anonymous_subject_rejected() {
        let syntax = parse(r#"{ "_": { "com.app.demo": { "x": 1 } } }"#);
        let err = interpret(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, InterpretError::AnonymousSubject));
    }
}
