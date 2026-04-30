//! Analyzer — turns a [`tonk_notation::Syntax`] tree into an
//! [`Analysis`] ready for evaluation against a branch.
//!
//! Current scope: a single expression per document (multi-
//! statement plans come later). Three shapes are recognised:
//!
//! - **Queries** (`head:` form). Resolves the head to a built-in
//!   or user-defined concept (via [`Resolver`]) — or recognises
//!   it as a claim domain — and produces a [`QueryPlan`] with a
//!   synthetic [`ConceptDescriptor`] plus per-field parameter
//!   bindings (literal / variable / blank).
//! - **Assertions** (`head!:` form). Resolves the descriptor
//!   the same way, derives the entity from the head's binding,
//!   and produces a [`TransactionPlan`] of resolved
//!   [`ClaimAssertion`]s. Built-in meta heads
//!   (`attribute!`, `concept!`) take a bespoke path that builds
//!   a real [`AttributeDescriptor`] / [`ConceptDescriptor`]
//!   from the body, derives the entity from the descriptor's
//!   identity, and binds the bookmark name (when given) via
//!   `dialog.meta/name`.
//! - **Retractions** (`head! …: _` form). Not yet implemented —
//!   they need a query-then-retract roundtrip to discover the
//!   entity's current values to dissociate against.
//!
//! See `tonk-notation/guide.md` for the user-facing notation
//! reference.

use std::collections::BTreeMap;

use async_trait::async_trait;
use dialog_artifacts::{Attribute as ArtifactsAttribute, Entity, Value};
use dialog_query::{AttributeDescriptor, ConceptDescriptor};
use thiserror::Error;
use tonk_notation::{
    Assertion, Binding, Expression, Field, FieldValue, HeadName, Reference, Scalar, Syntax,
};

/// Result of analyzing a [`Syntax`] tree — either a query plan
/// (read against the branch) or a transaction plan (write
/// against the branch). v1 supports exactly one expression per
/// document; multi-statement plans come later.
#[derive(Debug, Clone)]
pub enum Analysis {
    /// A read query, ready to evaluate against a dialog
    /// environment.
    Query(QueryPlan),
    /// A transaction, ready to commit against a branch.
    Transaction(TransactionPlan),
}

/// Plan for a single-statement query.
///
/// Carries the synthetic [`ConceptDescriptor`] (built from the
/// head concept's attributes), the parameter bindings (field
/// name → literal / variable / blank), and metadata for
/// rendering results.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// Built from the head's `with` map. Field names are
    /// preserved exactly as the concept defines them, even if the
    /// query body referenced only a subset; unmentioned fields
    /// are left as anonymous blanks so they're returned with the
    /// match without constraining it.
    pub descriptor: ConceptDescriptor,
    /// Per-field parameter shape — what to substitute into the
    /// descriptor query at evaluation time.
    pub parameters: Vec<ParameterBinding>,
    /// Display name for the head — used to label the rendered
    /// result block (`person { … }`).
    pub head_label: String,
    /// Optional named binding on the head itself
    /// (`person ?alice` → `Some("alice")`).
    pub head_variable: Option<String>,
}

/// Plan for a single-statement transaction (an assertion).
///
/// Carries the resolved facts to assert. Callers commit by
/// turning each [`ClaimAssertion`] into a dialog
/// [`dialog_artifacts::Statement`] — see the worker's
/// `transact` route for the canonical wiring.
///
/// v1 supports assertion only. Retractions need the entity's
/// current attribute values to dissociate against, which means
/// a query-then-retract roundtrip; that lands later.
#[derive(Debug, Clone)]
pub struct TransactionPlan {
    /// One assertion per facts to write. The same entity may
    /// appear in multiple assertions (one per attribute).
    pub assertions: Vec<ClaimAssertion>,
    /// Display name for the head — used by the worker to label
    /// the response (`person { … }`).
    pub head_label: String,
    /// The entity URIs the transaction touches (one per
    /// expression head). Surfaced so the worker can echo back
    /// "this is what got written" — useful when the user wrote
    /// an anonymous head and wants to know the minted entity.
    pub head_entity: Entity,
}

/// One assertion to commit — `(the, of, is)` plus the surface
/// field name for diagnostics and response labelling.
#[derive(Debug, Clone)]
pub struct ClaimAssertion {
    /// Attribute being written.
    pub the: ArtifactsAttribute,
    /// Entity the claim is about.
    pub of: Entity,
    /// Value to associate.
    pub is: Value,
    /// Field name as the user wrote it (matches a key in the
    /// concept's `with` map or the literal field name on a
    /// claim head). Used by the worker for response shaping.
    pub field_name: String,
}

/// One field's contribution to the synthetic concept query.
#[derive(Debug, Clone)]
pub struct ParameterBinding {
    /// Field name as defined by the concept (matches a key in
    /// [`ConceptDescriptor::with`]).
    pub name: String,
    /// What to substitute for the field — a literal value, a
    /// named variable, or a blank.
    pub value: ParameterValue,
}

/// Substitution shape for a query field.
#[derive(Debug, Clone)]
pub enum ParameterValue {
    /// User wrote a literal — match the field exactly.
    Literal(Scalar),
    /// User wrote `?var` — bind whatever matches under this name.
    Variable(String),
    /// User wrote `_` (or didn't mention the field at all) —
    /// return whatever matches without exposing it as a join key.
    Blank,
}

/// Errors raised while analyzing a [`Syntax`] tree.
#[derive(Debug, Error)]
pub enum AnalyzeError {
    /// Document has more than one expression — v0 only supports
    /// single-statement queries.
    #[error(
        "v0 supports exactly one expression per document; got {count}. \
         Multi-statement joins land in v1."
    )]
    MultipleExpressions {
        /// How many expressions were in the document.
        count: usize,
    },
    /// Document has zero expressions.
    #[error("document is empty — nothing to analyze")]
    EmptyDocument,
    /// Retraction expression — not yet implemented (needs a
    /// query+retract roundtrip to discover the entity's current
    /// values to dissociate against).
    #[error(
        "retractions (`head! …: _` or `field: _`) aren't supported \
         yet — they need a query+retract roundtrip we haven't wired"
    )]
    RetractionNotYetImplemented,
    /// Assertion head used a binding form we don't support yet.
    #[error(
        "assertion head with binding {form} isn't supported yet — \
         use an anonymous head (`{head}!:`) or an explicit URI \
         (`{head}! did:key:zX:`)"
    )]
    UnsupportedAssertionBinding {
        /// What kind of binding the user used.
        form: &'static str,
        /// The head name (without `!`), echoed for the suggested
        /// fix in the error message.
        head: String,
    },
    /// Assertion field value used a form we don't support yet
    /// (variable, blank, reference, nested).
    #[error(
        "assertion field {field:?} value {form} isn't supported \
         yet — use a literal scalar (string, number, boolean) for \
         now"
    )]
    UnsupportedAssertionFieldValue {
        /// Field where the offending value appeared.
        field: String,
        /// What kind of value it was.
        form: &'static str,
    },
    /// Assertion subject URI didn't parse as an entity.
    #[error("assertion subject {subject:?} is not a valid entity URI: {reason}")]
    InvalidSubjectUri {
        /// The subject text the user wrote.
        subject: String,
        /// Underlying parse error.
        reason: String,
    },
    /// Assertion body had no fields — nothing to write.
    #[error("assertion `{head}!` has no fields — at least one is required")]
    AssertionWithoutFields {
        /// The head name (without `!`).
        head: String,
    },
    /// `attribute!` body was malformed (missing `the`, invalid
    /// `as`/`cardinality` value, etc.).
    #[error("invalid `attribute!` body: {reason}")]
    InvalidAttributeBody {
        /// Underlying validation message.
        reason: String,
    },
    /// `concept!` body was malformed (missing `with:`, invalid
    /// reference, etc.).
    #[error("invalid `concept!` body: {reason}")]
    InvalidConceptBody {
        /// Underlying validation message.
        reason: String,
    },
    /// Head's concept name didn't resolve to anything known.
    #[error("unknown concept {name:?}: not a built-in and not found on the branch")]
    UnknownConcept {
        /// The concept name that failed to resolve.
        name: String,
    },
    /// A field in the body doesn't appear in the head concept's
    /// `with` map.
    #[error("field {field:?} is not part of concept {concept:?}")]
    UnknownField {
        /// The concept whose schema we were checking against.
        concept: String,
        /// The field name the user wrote.
        field: String,
    },
    /// A reference in field-value position couldn't be resolved.
    #[error(
        "field {field:?} references unknown bookmark {bookmark:?} \
         — define it earlier in the document or as an attribute on the branch"
    )]
    UnknownBookmark {
        /// Field where the reference appeared.
        field: String,
        /// The bookmark name.
        bookmark: String,
    },
    /// Claim head with no body fields — claims have no schema
    /// to fall back on, so the body must enumerate at least one
    /// field for the engine to query against.
    #[error(
        "claim head `{domain}:` needs at least one field. \
         Claims have no schema, so the parser cannot infer which \
         attributes to look up. Add the field names you want, \
         e.g. `{domain}:\\n  name: ?name`"
    )]
    ClaimWithoutFields {
        /// The claim domain.
        domain: String,
    },
    /// Claim attribute URI (`<domain>/<field>`) failed dialog's
    /// `the:…` validation (invalid characters, length cap, etc.).
    #[error("invalid attribute {domain:?}/{field:?}: {reason}")]
    InvalidClaimAttribute {
        /// The claim domain.
        domain: String,
        /// The field name.
        field: String,
        /// Underlying validation message.
        reason: String,
    },
    /// Resolver I/O failed.
    #[error("resolver error for {context}: {reason}")]
    ResolverFailed {
        /// What was being resolved.
        context: String,
        /// Underlying message.
        reason: String,
    },
}

/// An attribute resolved from outside the current document — its
/// entity URI plus the descriptor that produced it.
#[derive(Debug, Clone)]
pub struct ResolvedAttribute {
    /// The attribute's entity URI.
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: AttributeDescriptor,
}

/// A concept resolved from the branch — its entity URI plus the
/// reconstructed descriptor (so we can look up field-name →
/// attribute mappings without re-querying).
#[derive(Debug, Clone)]
pub struct ResolvedConcept {
    /// The concept entity URI (`concept:…`).
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: ConceptDescriptor,
}

/// Look up names against a backing store (typically the branch).
///
/// The analyzer calls this when it encounters a concept name in
/// head position or a bookmark reference in field-value position.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Resolver {
    /// Resolve a concept by name (or `Ok(None)` if not found).
    async fn resolve_concept(&self, name: &str) -> Result<Option<ResolvedConcept>, ResolverError>;

    /// Resolve an attribute by bookmark name. Used for field-
    /// value references (`field: .person-name`) and historically
    /// by the transaction interpreter — kept on the trait so the
    /// follow-up transaction work doesn't have to re-add it.
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError>;

    /// Resolve an attribute by its entity URI.
    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError>;
}

/// Opaque error from a [`Resolver`] implementation. The analyzer
/// wraps this into [`AnalyzeError::ResolverFailed`].
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
/// document-only analysis paths and unit tests.
pub struct NoopResolver;

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Resolver for NoopResolver {
    async fn resolve_concept(&self, _name: &str) -> Result<Option<ResolvedConcept>, ResolverError> {
        Ok(None)
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
}

/// Analyze a [`Syntax`] tree into an [`Analysis`].
///
/// v0 supports exactly one query expression per document.
/// Assertions, retractions, and multi-statement joins return
/// errors.
pub async fn analyze<R: Resolver>(syntax: &Syntax, resolver: &R) -> Result<Analysis, AnalyzeError> {
    if syntax.expressions.is_empty() {
        return Err(AnalyzeError::EmptyDocument);
    }
    if syntax.expressions.len() > 1 {
        return Err(AnalyzeError::MultipleExpressions {
            count: syntax.expressions.len(),
        });
    }

    let expression = &syntax.expressions[0];
    match expression {
        Expression::Query(q) => Ok(Analysis::Query(analyze_query(q, resolver).await?)),
        Expression::Assertion(a) => {
            Ok(Analysis::Transaction(analyze_assertion(a, resolver).await?))
        }
        Expression::Retraction(_) => Err(AnalyzeError::RetractionNotYetImplemented),
    }
}

async fn analyze_query<R: Resolver>(
    query: &tonk_notation::Query,
    resolver: &R,
) -> Result<QueryPlan, AnalyzeError> {
    let (descriptor, head_label) = match &query.head.name {
        HeadName::Concept(name) => {
            let resolved = resolver
                .resolve_concept(name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept { name: name.clone() })?;
            (resolved.descriptor, name.clone())
        }
        HeadName::Claim(domain) => {
            // Claim heads have no schema on the branch; build a
            // synthetic descriptor on the spot from the user's
            // body fields. Each field becomes an attribute under
            // `<domain>/<field-name>` with no type constraint
            // (the engine accepts any value type when
            // `content_type` is `None`).
            let descriptor = build_claim_descriptor(domain, &query.fields)?;
            (descriptor, domain.clone())
        }
    };

    let parameters = analyze_query_fields(&head_label, &descriptor, &query.fields)?;

    let head_variable = match &query.head.binding {
        Binding::Variable(v) => Some(v.clone()),
        Binding::Anonymous | Binding::Bookmark(_) | Binding::Uri(_) => None,
    };

    Ok(QueryPlan {
        descriptor,
        parameters,
        head_label,
        head_variable,
    })
}

/// Analyze an assertion expression into a [`TransactionPlan`].
///
/// v1 scope:
/// - Head binding must be `Anonymous` (a fresh entity is minted)
///   or `Uri` (the user supplies an explicit `did:key:…`).
/// - Body fields must be literal scalars; variables, blanks,
///   references, and nested mappings are rejected.
/// - Concept heads validate fields against the resolved
///   descriptor's `with` map; claim heads accept any field name
///   and synthesize `<domain>/<field>` attributes.
/// - Built-in meta heads (`attribute!`, `concept!`) take a
///   bespoke path that builds a real
///   [`AttributeDescriptor`] / [`ConceptDescriptor`] from the
///   body, derives the entity from the descriptor's identity,
///   and binds the bookmark name (when given) via
///   `dialog.meta/name`.
async fn analyze_assertion<R: Resolver>(
    assertion: &Assertion,
    resolver: &R,
) -> Result<TransactionPlan, AnalyzeError> {
    // Built-in meta heads bypass the resolver and the
    // standard-assertion machinery — their schema is fixed and
    // their body shape doesn't fit the literal-fields-only mold
    // we apply elsewhere.
    if let HeadName::Concept(name) = &assertion.head.name {
        match name.as_str() {
            "attribute" => return analyze_attribute_assertion(assertion),
            "concept" => return analyze_concept_assertion(assertion, resolver).await,
            _ => {}
        }
    }

    let (descriptor, head_label) = match &assertion.head.name {
        HeadName::Concept(name) => {
            let resolved = resolver
                .resolve_concept(name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept { name: name.clone() })?;
            (resolved.descriptor, name.clone())
        }
        HeadName::Claim(domain) => {
            let descriptor = build_claim_descriptor(domain, &assertion.fields)?;
            (descriptor, domain.clone())
        }
    };

    if assertion.fields.is_empty() {
        return Err(AnalyzeError::AssertionWithoutFields { head: head_label });
    }

    // Resolve the entity the assertion writes against.
    let head_entity = match &assertion.head.binding {
        Binding::Anonymous => Entity::new().map_err(|e| AnalyzeError::ResolverFailed {
            context: "minting fresh entity".into(),
            reason: e.to_string(),
        })?,
        Binding::Uri(uri) => uri
            .parse()
            .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                AnalyzeError::InvalidSubjectUri {
                    subject: uri.clone(),
                    reason: e.to_string(),
                }
            })?,
        Binding::Variable(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "variable (`?name`)",
                head: head_label,
            });
        }
        Binding::Bookmark(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "bookmark name",
                head: head_label,
            });
        }
    };

    // Walk user-supplied fields, look each up in the
    // descriptor, build a (the, of, is) triple.
    let mut user_fields: BTreeMap<&str, &FieldValue> = BTreeMap::new();
    for field in &assertion.fields {
        user_fields.insert(field.name.as_str(), &field.value);
    }

    let mut assertions = Vec::new();
    for (field_name, attribute) in descriptor.with().iter() {
        let Some(field_value) = user_fields.remove(field_name) else {
            // Field is in the concept's `with` but the user
            // didn't supply it. v1 silently skips — partial
            // assertions are valid (you might be filling in
            // half a concept). Future work could add a
            // concept-level "all-or-nothing" mode.
            continue;
        };
        let scalar = match field_value {
            FieldValue::Literal(s) => s,
            FieldValue::Variable(_) => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field_name.to_owned(),
                    form: "variable (`?name`)",
                });
            }
            FieldValue::Blank => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field_name.to_owned(),
                    form: "blank (`_`) — field-level retraction needs query+retract",
                });
            }
            FieldValue::Reference(_) => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field_name.to_owned(),
                    form: "reference (`.bookmark` / URI)",
                });
            }
            FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field_name.to_owned(),
                    form: "nested mapping",
                });
            }
        };
        let value = scalar_to_value(scalar)?;
        let the: ArtifactsAttribute = attribute.the().clone().into();
        assertions.push(ClaimAssertion {
            the,
            of: head_entity.clone(),
            is: value,
            field_name: field_name.to_owned(),
        });
    }

    if let Some((unknown, _)) = user_fields.into_iter().next() {
        return Err(AnalyzeError::UnknownField {
            concept: head_label,
            field: unknown.to_owned(),
        });
    }

    Ok(TransactionPlan {
        assertions,
        head_label,
        head_entity,
    })
}

/// Analyze an `attribute! <bookmark>:` (or `attribute!:`)
/// assertion. Builds a real [`AttributeDescriptor`] from the
/// body fields, derives the entity from the descriptor's
/// canonical URI, and emits the indexable facts:
///
/// - `dialog.attribute/id` (the `domain/name` selector)
/// - `dialog.attribute/type` (the value type, when set)
/// - `dialog.attribute/cardinality` (`one` / `many`)
/// - `dialog.meta/description` (when set)
/// - `dialog.meta/name` (when the head carries a bookmark
///   binding so the name can later resolve back to the entity)
fn analyze_attribute_assertion(assertion: &Assertion) -> Result<TransactionPlan, AnalyzeError> {
    let bookmark = match &assertion.head.binding {
        Binding::Anonymous => None,
        Binding::Bookmark(name) => Some(name.clone()),
        Binding::Variable(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "variable (`?name`)",
                head: "attribute".into(),
            });
        }
        Binding::Uri(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "URI — attribute identity is content-derived",
                head: "attribute".into(),
            });
        }
    };

    // Pull body fields by name. `attribute!` accepts a fixed
    // schema (the / as / cardinality / description); anything
    // else is a typo.
    let mut shape = serde_json::Map::new();
    for field in &assertion.fields {
        let value_str = match &field.value {
            FieldValue::Literal(Scalar::String(s)) => s.clone(),
            FieldValue::Literal(other) => {
                // Numbers/bools coerce to their string form so
                // dialog's `Type::deserialize` can still read
                // them — the schema fields are all string-typed
                // on the wire.
                serde_json::to_value(scalar_to_value(other)?)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            }
            FieldValue::Reference(Reference::Uri(s)) => s.clone(),
            FieldValue::Reference(Reference::Bookmark(_)) => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field.name.clone(),
                    form: "bookmark reference (`attribute!` body must be literals)",
                });
            }
            FieldValue::Variable(_) | FieldValue::Blank | FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                    field: field.name.clone(),
                    form: "non-literal (`attribute!` body must be literals)",
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

    let mut assertions = Vec::new();
    let id_selector = format!("{}/{}", descriptor.domain(), descriptor.name());
    assertions.push(ClaimAssertion {
        the: meta_attr("dialog.attribute", "id"),
        of: entity.clone(),
        is: Value::String(id_selector),
        field_name: "id".into(),
    });
    if let Some(ty) = descriptor.content_type() {
        let type_name = serde_json::to_value(ty)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if !type_name.is_empty() {
            assertions.push(ClaimAssertion {
                the: meta_attr("dialog.attribute", "type"),
                of: entity.clone(),
                is: Value::String(type_name),
                field_name: "type".into(),
            });
        }
    }
    let cardinality_name = serde_json::to_value(descriptor.cardinality())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "one".into());
    assertions.push(ClaimAssertion {
        the: meta_attr("dialog.attribute", "cardinality"),
        of: entity.clone(),
        is: Value::String(cardinality_name),
        field_name: "cardinality".into(),
    });
    let description = descriptor.description();
    if !description.is_empty() {
        assertions.push(ClaimAssertion {
            the: meta_attr("dialog.meta", "description"),
            of: entity.clone(),
            is: Value::String(description.to_owned()),
            field_name: "description".into(),
        });
    }
    if let Some(name) = bookmark {
        assertions.push(ClaimAssertion {
            the: meta_attr("dialog.meta", "name"),
            of: entity.clone(),
            is: Value::String(name),
            field_name: "name".into(),
        });
    }

    Ok(TransactionPlan {
        assertions,
        head_label: "attribute".into(),
        head_entity: entity,
    })
}

/// Analyze a `concept! <bookmark>:` (or `concept!:`) assertion.
///
/// Builds a [`ConceptDescriptor`] by walking the `with:` map
/// (each value is a `.bookmark` reference resolved against the
/// branch via the resolver), derives the entity from the
/// descriptor's content-addressed identity, and emits:
///
/// - `dialog.concept.with/{name}` for each field (value =
///   referenced attribute entity)
/// - `dialog.meta/description` (when set)
/// - `dialog.meta/name` (when the head carries a bookmark
///   binding)
async fn analyze_concept_assertion<R: Resolver>(
    assertion: &Assertion,
    resolver: &R,
) -> Result<TransactionPlan, AnalyzeError> {
    let bookmark = match &assertion.head.binding {
        Binding::Anonymous => None,
        Binding::Bookmark(name) => Some(name.clone()),
        Binding::Variable(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "variable (`?name`)",
                head: "concept".into(),
            });
        }
        Binding::Uri(_) => {
            return Err(AnalyzeError::UnsupportedAssertionBinding {
                form: "URI — concept identity is content-derived",
                head: "concept".into(),
            });
        }
    };

    let mut description: Option<String> = None;
    let mut with_fields: Vec<(String, ResolvedAttribute)> = Vec::new();

    for field in &assertion.fields {
        match field.name.as_str() {
            "description" => {
                if let FieldValue::Literal(Scalar::String(s)) = &field.value {
                    description = Some(s.clone());
                } else {
                    return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                        field: "description".into(),
                        form: "non-string literal",
                    });
                }
            }
            "with" => {
                let FieldValue::Nested(inner) = &field.value else {
                    return Err(AnalyzeError::InvalidConceptBody {
                        reason: "`with:` must be a mapping of field name → \
                                 attribute reference (`.bookmark` or URI)"
                            .into(),
                    });
                };
                for sub in inner {
                    let resolved = resolve_concept_field(&sub.name, &sub.value, resolver).await?;
                    with_fields.push((sub.name.clone(), resolved));
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

    // Build the descriptor through serde so we don't have to
    // mirror dialog's internal `with` shape.
    let mut shape = serde_json::Map::new();
    if let Some(desc) = &description {
        shape.insert(
            "description".into(),
            serde_json::Value::String(desc.clone()),
        );
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

    let mut assertions = Vec::new();
    if let Some(desc) = &description {
        assertions.push(ClaimAssertion {
            the: meta_attr("dialog.meta", "description"),
            of: entity.clone(),
            is: Value::String(desc.clone()),
            field_name: "description".into(),
        });
    }
    for (name, attr) in &with_fields {
        let relation = meta_attr("dialog.concept.with", name);
        assertions.push(ClaimAssertion {
            the: relation,
            of: entity.clone(),
            is: Value::Entity(attr.entity.clone()),
            field_name: name.clone(),
        });
    }
    if let Some(name) = bookmark {
        assertions.push(ClaimAssertion {
            the: meta_attr("dialog.meta", "name"),
            of: entity.clone(),
            is: Value::String(name),
            field_name: "name".into(),
        });
    }

    Ok(TransactionPlan {
        assertions,
        head_label: "concept".into(),
        head_entity: entity,
    })
}

/// Resolve a single `with:` entry's value to a [`ResolvedAttribute`].
async fn resolve_concept_field<R: Resolver>(
    field_name: &str,
    value: &FieldValue,
    resolver: &R,
) -> Result<ResolvedAttribute, AnalyzeError> {
    match value {
        FieldValue::Reference(Reference::Bookmark(name)) => resolver
            .resolve_attribute(name)
            .await
            .map_err(|e| AnalyzeError::ResolverFailed {
                context: format!("attribute bookmark {name:?}"),
                reason: e.message,
            })?
            .ok_or_else(|| AnalyzeError::UnknownBookmark {
                field: field_name.into(),
                bookmark: name.clone(),
            }),
        FieldValue::Reference(Reference::Uri(uri)) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeError::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            resolver
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
        FieldValue::Literal(Scalar::String(s)) => {
            // Lenient fallback: a bare-string value is treated
            // as a bookmark name. Lets users write
            // `name: person-name` instead of `name: .person-name`.
            resolver
                .resolve_attribute(s)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("attribute bookmark {s:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownBookmark {
                    field: field_name.into(),
                    bookmark: s.clone(),
                })
        }
        _ => Err(AnalyzeError::UnsupportedAssertionFieldValue {
            field: field_name.into(),
            form: "expected `.bookmark` reference or attribute URI",
        }),
    }
}

/// Build a runtime [`ArtifactsAttribute`] from a domain + local
/// name. Both halves are validated by dialog's own parser; we
/// surface failures as [`AnalyzeError::InvalidClaimAttribute`].
fn meta_attr(domain: &str, name: &str) -> ArtifactsAttribute {
    format!("{domain}/{name}")
        .parse()
        .expect("dialog meta-attribute names should always be valid")
}

/// Convert a notation [`Scalar`] into a dialog [`Value`].
fn scalar_to_value(scalar: &Scalar) -> Result<Value, AnalyzeError> {
    Ok(match scalar {
        Scalar::String(s) => Value::String(s.clone()),
        Scalar::Boolean(b) => Value::Boolean(*b),
        Scalar::Integer(i) => Value::SignedInt(*i),
        Scalar::UnsignedInteger(u) => Value::UnsignedInt(*u),
        Scalar::Float(f) => Value::Float(*f),
        Scalar::Null => {
            return Err(AnalyzeError::UnsupportedAssertionFieldValue {
                field: "<scalar>".into(),
                form: "null literal",
            });
        }
    })
}

/// Synthesize a [`ConceptDescriptor`] on the fly for a claim
/// head. Each user-supplied field becomes a `with` attribute
/// under `<domain>/<field-name>`; cardinality defaults to `one`
/// and the value type is left unconstrained.
fn build_claim_descriptor(
    domain: &str,
    fields: &[Field],
) -> Result<ConceptDescriptor, AnalyzeError> {
    use dialog_query::attribute::{Cardinality, The};

    if fields.is_empty() {
        return Err(AnalyzeError::ClaimWithoutFields {
            domain: domain.to_owned(),
        });
    }

    let mut entries: Vec<(String, AttributeDescriptor)> = Vec::with_capacity(fields.len());
    for field in fields {
        let uri = format!("{domain}/{name}", name = field.name);
        let the: The = uri
            .parse()
            .map_err(|e| AnalyzeError::InvalidClaimAttribute {
                domain: domain.to_owned(),
                field: field.name.clone(),
                reason: format!("{e}"),
            })?;
        entries.push((
            field.name.clone(),
            AttributeDescriptor::new(the, "", Cardinality::default(), None),
        ));
    }

    Ok(ConceptDescriptor::from(entries))
}

fn analyze_query_fields(
    concept_name: &str,
    descriptor: &ConceptDescriptor,
    fields: &[Field],
) -> Result<Vec<ParameterBinding>, AnalyzeError> {
    // Index the user-supplied fields by name for O(1) lookup
    // while we walk the descriptor's `with` map in canonical
    // order.
    let mut user_fields: BTreeMap<&str, &FieldValue> = BTreeMap::new();
    for field in fields {
        user_fields.insert(field.name.as_str(), &field.value);
    }

    // Walk the concept's `with` map. Every field of the concept
    // becomes a parameter — even ones the user didn't mention
    // (those become anonymous blanks so the value is matched
    // without constraining it).
    let mut parameters = Vec::new();
    for (field_name, _attribute_descriptor) in descriptor.with().iter() {
        let value = match user_fields.remove(field_name) {
            Some(FieldValue::Literal(scalar)) => ParameterValue::Literal(scalar.clone()),
            Some(FieldValue::Variable(name)) => ParameterValue::Variable(name.clone()),
            Some(FieldValue::Blank) | None => ParameterValue::Blank,
            Some(FieldValue::Reference(Reference::Bookmark(name))) => {
                // v0: bookmarks in field position aren't resolved
                // yet — surface as an error so the user knows.
                return Err(AnalyzeError::UnknownBookmark {
                    field: field_name.to_owned(),
                    bookmark: name.clone(),
                });
            }
            Some(FieldValue::Reference(Reference::Uri(uri))) => {
                // Treat URI references as literal-string matches
                // for v0. This works for `Entity`-typed fields
                // and round-trips on the engine side.
                ParameterValue::Literal(Scalar::String(uri.clone()))
            }
            Some(FieldValue::Nested(_)) => {
                // Nested mappings in query position aren't part
                // of v0; reject for now.
                return Err(AnalyzeError::UnknownField {
                    concept: concept_name.to_owned(),
                    field: field_name.to_owned(),
                });
            }
        };
        parameters.push(ParameterBinding {
            name: field_name.to_owned(),
            value,
        });
    }

    // Anything left in `user_fields` was a name the concept
    // doesn't know about.
    if let Some((unknown, _)) = user_fields.into_iter().next() {
        return Err(AnalyzeError::UnknownField {
            concept: concept_name.to_owned(),
            field: unknown.to_owned(),
        });
    }

    Ok(parameters)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);
    use tonk_notation::parse;

    /// Build a `ConceptDescriptor` with the given fields for tests.
    fn make_concept_descriptor(fields: &[(&str, &str)]) -> ConceptDescriptor {
        let mut with = serde_json::Map::new();
        for (name, the) in fields {
            with.insert(
                (*name).to_owned(),
                serde_json::json!({ "the": the, "as": "Text" }),
            );
        }
        let json = serde_json::json!({ "with": with });
        serde_json::from_value(json).unwrap()
    }

    /// Resolver that returns a fixed concept descriptor for a
    /// given name; otherwise nothing.
    struct FixedResolver {
        name: String,
        concept: ResolvedConcept,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Resolver for FixedResolver {
        async fn resolve_concept(
            &self,
            name: &str,
        ) -> Result<Option<ResolvedConcept>, ResolverError> {
            if name == self.name {
                Ok(Some(self.concept.clone()))
            } else {
                Ok(None)
            }
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
    }

    fn fixed_resolver(name: &str, fields: &[(&str, &str)]) -> FixedResolver {
        let descriptor = make_concept_descriptor(fields);
        let entity = descriptor.this();
        FixedResolver {
            name: name.to_owned(),
            concept: ResolvedConcept { entity, descriptor },
        }
    }

    /// Pull the query plan out of an `Analysis`, panicking with
    /// the actual variant if it wasn't a query.
    fn expect_query(analysis: Analysis) -> QueryPlan {
        match analysis {
            Analysis::Query(q) => q,
            other => panic!("expected Query, got {other:?}"),
        }
    }

    /// Pull the transaction plan out of an `Analysis`,
    /// panicking with the actual variant if it wasn't one.
    fn expect_transaction(analysis: Analysis) -> TransactionPlan {
        match analysis {
            Analysis::Transaction(t) => t,
            other => panic!("expected Transaction, got {other:?}"),
        }
    }

    #[dialog_common::test]
    async fn empty_document_is_an_error() {
        // Construct a Syntax with zero expressions directly. In
        // practice the worker filters Parsed.syntax = None before
        // calling analyze, but a Syntax with an empty expression
        // list is still possible (e.g. through future builder
        // APIs), so analyze must reject it.
        let syntax = Syntax {
            expressions: Vec::new(),
            range: lsp_types::Range::default(),
        };
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::EmptyDocument));
    }

    #[dialog_common::test]
    async fn assertion_with_uri_binding_works() {
        let syntax = parse(
            "person! did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv:\n\
             \x20 name: \"Alice\"\n\
             \x20 age: 28\n",
        )
        .syntax
        .unwrap();
        let resolver = fixed_resolver(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let plan = expect_transaction(analyze(&syntax, &resolver).await.unwrap());
        assert_eq!(plan.head_label, "person");
        assert_eq!(plan.assertions.len(), 2);
        assert!(
            plan.assertions.iter().any(
                |a| a.field_name == "name" && matches!(&a.is, Value::String(s) if s == "Alice")
            )
        );
    }

    #[dialog_common::test]
    async fn assertion_with_anonymous_binding_mints_entity() {
        let syntax = parse("person!:\n  name: \"Alice\"\n").syntax.unwrap();
        let resolver = fixed_resolver("person", &[("name", "io.gozala.person/name")]);
        let plan = expect_transaction(analyze(&syntax, &resolver).await.unwrap());
        // Two anonymous heads of the same shape should mint
        // different entities — `Entity::new` is random, not
        // content-derived.
        let plan2 = expect_transaction(analyze(&syntax, &resolver).await.unwrap());
        assert_ne!(plan.head_entity, plan2.head_entity);
    }

    #[dialog_common::test]
    async fn assertion_with_variable_binding_is_unsupported() {
        let syntax = parse("person! ?alice:\n  name: \"Alice\"\n")
            .syntax
            .unwrap();
        let resolver = fixed_resolver("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err,
            AnalyzeError::UnsupportedAssertionBinding { .. }
        ));
    }

    #[dialog_common::test]
    async fn assertion_with_blank_field_is_unsupported() {
        let syntax = parse(
            "person! did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv:\n\
             \x20 name: _\n",
        )
        .syntax
        .unwrap();
        let resolver = fixed_resolver("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(
            err,
            AnalyzeError::UnsupportedAssertionFieldValue { .. }
        ));
    }

    #[dialog_common::test]
    async fn attribute_assertion_emits_meta_facts() {
        let syntax = parse(
            "attribute! person-name:\n\
             \x20 description: \"The person's name\"\n\
             \x20 the:         io.gozala.person/name\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n",
        )
        .syntax
        .unwrap();
        let plan = expect_transaction(analyze(&syntax, &NoopResolver).await.unwrap());
        assert_eq!(plan.head_label, "attribute");
        // Should emit id + type + cardinality + description + name
        // (5 claims for a fully-specified attribute with a
        // bookmark binding).
        let by_field: BTreeMap<&str, &ClaimAssertion> = plan
            .assertions
            .iter()
            .map(|c| (c.field_name.as_str(), c))
            .collect();
        assert!(matches!(
            &by_field["id"].is,
            Value::String(s) if s == "io.gozala.person/name"
        ));
        assert!(matches!(
            &by_field["type"].is,
            Value::String(s) if s == "Text"
        ));
        assert!(matches!(
            &by_field["cardinality"].is,
            Value::String(s) if s == "one"
        ));
        assert!(matches!(
            &by_field["description"].is,
            Value::String(s) if s == "The person's name"
        ));
        assert!(matches!(
            &by_field["name"].is,
            Value::String(s) if s == "person-name"
        ));
    }

    #[dialog_common::test]
    async fn attribute_assertion_without_the_is_an_error() {
        let syntax = parse("attribute! foo:\n  as: Text\n").syntax.unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::InvalidAttributeBody { .. }));
    }

    #[dialog_common::test]
    async fn concept_assertion_resolves_field_references() {
        // Two attributes need to resolve to build the concept.
        struct TwoAttributesResolver;
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        impl Resolver for TwoAttributesResolver {
            async fn resolve_concept(
                &self,
                _name: &str,
            ) -> Result<Option<ResolvedConcept>, ResolverError> {
                Ok(None)
            }
            async fn resolve_attribute(
                &self,
                name: &str,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                let the = match name {
                    "person-name" => "io.gozala.person/name",
                    "person-age" => "io.gozala.person/age",
                    _ => return Ok(None),
                };
                let descriptor: AttributeDescriptor = serde_json::from_value(serde_json::json!({
                    "the": the, "as": "Text",
                }))
                .unwrap();
                let entity: Entity = descriptor.to_uri().parse().unwrap();
                Ok(Some(ResolvedAttribute { entity, descriptor }))
            }
            async fn resolve_attribute_by_entity(
                &self,
                _entity: &Entity,
            ) -> Result<Option<ResolvedAttribute>, ResolverError> {
                Ok(None)
            }
        }

        let syntax = parse(
            "concept! person:\n\
             \x20 description: A person\n\
             \x20 with:\n\
             \x20   name: .person-name\n\
             \x20   age:  .person-age\n",
        )
        .syntax
        .unwrap();
        let plan = expect_transaction(analyze(&syntax, &TwoAttributesResolver).await.unwrap());
        assert_eq!(plan.head_label, "concept");
        // Should emit description + 2 with/{field} + name
        // = 4 claims.
        assert_eq!(plan.assertions.len(), 4);
        let with_fields: Vec<&str> = plan
            .assertions
            .iter()
            .filter(|c| c.field_name == "name" || c.field_name == "age")
            .map(|c| c.field_name.as_str())
            .collect();
        // Name field appears twice — once for `dialog.meta/name`
        // and once for `dialog.concept.with/name`.
        assert!(with_fields.contains(&"age"));
    }

    #[dialog_common::test]
    async fn concept_assertion_without_with_is_an_error() {
        let syntax = parse(
            "concept! foo:\n\
             \x20 description: oops\n",
        )
        .syntax
        .unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::InvalidConceptBody { .. }));
    }

    #[dialog_common::test]
    async fn retraction_is_not_yet_implemented() {
        let syntax = parse("person! did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv: _\n")
            .syntax
            .unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::RetractionNotYetImplemented));
    }

    #[dialog_common::test]
    async fn unknown_concept_errors() {
        let syntax = parse("nope:\n").syntax.unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnknownConcept { .. }));
    }

    #[dialog_common::test]
    async fn query_with_literal_and_variable() {
        let syntax = parse(
            "person ?alice:\n\
             \x20 name: \"Alice\"\n\
             \x20 age: ?age\n",
        )
        .syntax
        .unwrap();
        let resolver = fixed_resolver(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let plan = expect_query(analyze(&syntax, &resolver).await.unwrap());
        assert_eq!(plan.head_label, "person");
        assert_eq!(plan.head_variable.as_deref(), Some("alice"));
        assert_eq!(plan.parameters.len(), 2);
        let by_name: BTreeMap<&str, &ParameterValue> = plan
            .parameters
            .iter()
            .map(|p| (p.name.as_str(), &p.value))
            .collect();
        assert!(matches!(
            by_name["name"],
            ParameterValue::Literal(Scalar::String(s)) if s == "Alice"
        ));
        assert!(matches!(
            by_name["age"],
            ParameterValue::Variable(s) if s == "age"
        ));
    }

    #[dialog_common::test]
    async fn unmentioned_fields_become_blanks() {
        let syntax = parse("person:\n  name: \"Alice\"\n").syntax.unwrap();
        let resolver = fixed_resolver(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let plan = expect_query(analyze(&syntax, &resolver).await.unwrap());
        let by_name: BTreeMap<&str, &ParameterValue> = plan
            .parameters
            .iter()
            .map(|p| (p.name.as_str(), &p.value))
            .collect();
        assert!(matches!(by_name["age"], ParameterValue::Blank));
    }

    #[dialog_common::test]
    async fn unknown_field_errors() {
        let syntax = parse("person:\n  bogus: \"x\"\n").syntax.unwrap();
        let resolver = fixed_resolver("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnknownField { .. }));
    }

    #[dialog_common::test]
    async fn claim_head_builds_synthetic_descriptor() {
        // No resolver call needed — the analyzer builds the
        // descriptor from the body fields directly.
        let syntax = parse("xyz.tonk:\n  role: ?role\n  contact: \"alice\"\n")
            .syntax
            .unwrap();
        let plan = expect_query(analyze(&syntax, &NoopResolver).await.unwrap());
        assert_eq!(plan.head_label, "xyz.tonk");
        assert_eq!(plan.parameters.len(), 2);
        let by_name: BTreeMap<&str, &ParameterValue> = plan
            .parameters
            .iter()
            .map(|p| (p.name.as_str(), &p.value))
            .collect();
        assert!(matches!(
            by_name["role"],
            ParameterValue::Variable(s) if s == "role"
        ));
        assert!(matches!(
            by_name["contact"],
            ParameterValue::Literal(Scalar::String(s)) if s == "alice"
        ));
    }

    #[dialog_common::test]
    async fn claim_head_without_fields_is_an_error() {
        let syntax = parse("xyz.tonk:\n").syntax.unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::ClaimWithoutFields { .. }));
    }

    #[dialog_common::test]
    async fn multiple_expressions_errors() {
        // Two heads with different names so the YAML mapping
        // doesn't deduplicate them.
        let syntax = parse("person:\n  name: \"A\"\nplace:\n  name: \"B\"\n")
            .syntax
            .unwrap();
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::MultipleExpressions { .. }));
    }
}
