//! Analyzer — turns a [`tonk_notation::Syntax`] tree into a
//! [`crate::transact::Analysis`] ready for evaluation against a
//! branch.
//!
//! See `analysis-spec.md` (sibling to this crate) for the full
//! design. Analysis runs in three phases:
//!
//! 1. **Derive.** Walk every head; populate `declarations`
//!    (bookmark-form heads) and `variables` (variable-form
//!    heads) with content-derived entities. For `attribute!` /
//!    `concept!` heads the body is parsed in this phase to
//!    compute the descriptor's content-addressed entity.
//! 2. **Build the query.** For each query expression, build an
//!    [`Application`] with `.bookmark` and analysis-time `?var`
//!    references substituted to constants.
//! 3. **Build the mutation plan.** For each mutation expression,
//!    build an [`Application`] with `.bookmark` references
//!    substituted but `?var` references kept as variables — they
//!    bind at planning time against query results.

use std::collections::{BTreeMap, HashMap, HashSet};

use async_trait::async_trait;
use dialog_artifacts::{Entity, Value};
use dialog_query::{
    AttributeDescriptor, ConceptDescriptor, Parameters, Term, attribute::The as AttributeThe,
    concept::query::ConceptQuery,
};
use thiserror::Error;
use tonk_notation::{
    Assertion, Binding, Expression, Field, FieldValue, HeadName, Reference, Retraction, Scalar,
    Syntax,
};

use crate::prelude::EntityExt;
use crate::transact::{
    Analysis, Application, DomainApplication, HeadBinding, MutationAnalysis, QueryAnalysis,
    Statement,
};

// ---------------------------------------------------------------- //
// Resolver — branch-side name lookup                               //
// ---------------------------------------------------------------- //

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
    /// value references (`field: .person-name`) and by
    /// `concept!`'s `with:` map.
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

// ---------------------------------------------------------------- //
// Errors                                                           //
// ---------------------------------------------------------------- //

/// Errors raised while analyzing a [`Syntax`] tree.
#[derive(Debug, Error)]
pub enum AnalyzeError {
    /// Document has zero expressions.
    #[error("document is empty — nothing to analyze")]
    EmptyDocument,
    /// Two heads in the document tried to declare the same
    /// `.bookmark` or `?variable` name.
    #[error(
        "name {name:?} declared twice — bookmarks and variables must be unique within a document"
    )]
    DuplicateName {
        /// The name that was duplicated.
        name: String,
    },
    /// A `.bookmark` or `?variable` is used by both a declaration
    /// and a variable head — they share the same namespace.
    #[error("name {name:?} is used as both a bookmark declaration and a variable — pick one")]
    NameShadowing {
        /// The conflicting name.
        name: String,
    },
    /// A mutation references `?var` that no source binds (neither
    /// analysis-time variables nor query bindings).
    #[error(
        "mutation references unbound variable ?{name} — define it earlier with `?{name}:` or bind it via a query"
    )]
    UnboundMutationVariable {
        /// The variable name.
        name: String,
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
    /// `concept!` body was malformed.
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
    /// Claim head with no body fields — claims have no schema to
    /// fall back on.
    #[error(
        "claim head `{domain}:` needs at least one field. \
         Claims have no schema, so the parser cannot infer which \
         attributes to look up. Add the field names you want, e.g. \
         `{domain}:\\n  name: ?name`"
    )]
    ClaimWithoutFields {
        /// The claim domain.
        domain: String,
    },
    /// Claim attribute URI failed dialog's `the:…` validation.
    #[error("invalid attribute {domain:?}/{field:?}: {reason}")]
    InvalidClaimAttribute {
        /// The claim domain.
        domain: String,
        /// The field name.
        field: String,
        /// Underlying validation message.
        reason: String,
    },
    /// Field value used a form the analyzer doesn't accept here.
    #[error("field {field:?} value {form} isn't supported here")]
    UnsupportedFieldValue {
        /// Field where the offending value appeared.
        field: String,
        /// What kind of value it was.
        form: &'static str,
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

// ---------------------------------------------------------------- //
// In-document overlay resolver                                     //
// ---------------------------------------------------------------- //

/// Cell that stays Send + Sync on native (so async-trait
/// futures hold across awaits in axum handlers) but stays
/// single-threaded on wasm (no need for Mutex).
#[cfg(not(target_arch = "wasm32"))]
type Cell<T> = std::sync::Mutex<T>;
#[cfg(target_arch = "wasm32")]
type Cell<T> = std::cell::RefCell<T>;

#[cfg(not(target_arch = "wasm32"))]
fn cell_borrow<T>(cell: &Cell<T>) -> std::sync::MutexGuard<'_, T> {
    cell.lock().expect("Scope mutex is never poisoned")
}
#[cfg(target_arch = "wasm32")]
fn cell_borrow<T>(cell: &Cell<T>) -> std::cell::Ref<'_, T> {
    cell.borrow()
}

#[cfg(not(target_arch = "wasm32"))]
fn cell_borrow_mut<T>(cell: &Cell<T>) -> std::sync::MutexGuard<'_, T> {
    cell.lock().expect("Scope mutex is never poisoned")
}
#[cfg(target_arch = "wasm32")]
fn cell_borrow_mut<T>(cell: &Cell<T>) -> std::cell::RefMut<'_, T> {
    cell.borrow_mut()
}

#[cfg(not(target_arch = "wasm32"))]
fn cell_new<T>(value: T) -> Cell<T> {
    std::sync::Mutex::new(value)
}
#[cfg(target_arch = "wasm32")]
fn cell_new<T>(value: T) -> Cell<T> {
    std::cell::RefCell::new(value)
}

/// Layered name index built during analysis. Phase 1 fills it in
/// (bookmark/variable → entity, plus `concept!` definitions for
/// later expressions in the same document); Phase 2 and 3 read
/// from it.
struct Scope<'a, R: Resolver> {
    inner: &'a R,
    /// Bookmark/variable → entity for non-meta head bindings
    /// (every head except `attribute!` / `concept!` whose
    /// declarations live in the dedicated maps below). One map
    /// per source — bookmark vs variable — surfaced separately
    /// because `Analysis` keeps them separate.
    declarations: Cell<HashMap<String, Entity>>,
    variables: Cell<HashMap<String, Entity>>,
    /// `attribute!` definitions made in the document, indexed by
    /// the bookmark/variable name on the head. Used by later
    /// `concept!` heads in the same document so their `with:`
    /// map can resolve `.bookmark` / `?var` references against
    /// uncommitted attributes.
    in_doc_attributes: Cell<HashMap<String, ResolvedAttribute>>,
    /// `concept!` definitions made in the document, indexed by
    /// the bookmark/variable name on the head. Used by later
    /// `person! alice:` heads in the same document.
    in_doc_concepts: Cell<HashMap<String, ResolvedConcept>>,
    /// Reverse index: attribute entity → resolved attribute.
    /// Used when a concept body references an attribute via URI
    /// instead of by name.
    in_doc_attributes_by_entity: Cell<HashMap<String, ResolvedAttribute>>,
    /// Reverse index: concept entity → resolved concept.
    in_doc_concepts_by_entity: Cell<HashMap<String, ResolvedConcept>>,
}

impl<'a, R: Resolver> Scope<'a, R> {
    fn new(inner: &'a R) -> Self {
        Self {
            inner,
            declarations: cell_new(HashMap::new()),
            variables: cell_new(HashMap::new()),
            in_doc_attributes: cell_new(HashMap::new()),
            in_doc_concepts: cell_new(HashMap::new()),
            in_doc_attributes_by_entity: cell_new(HashMap::new()),
            in_doc_concepts_by_entity: cell_new(HashMap::new()),
        }
    }

    /// Record a bookmark-form head's entity.
    fn declare(&self, name: &str, entity: Entity) -> Result<(), AnalyzeError> {
        if cell_borrow(&self.variables).contains_key(name) {
            return Err(AnalyzeError::NameShadowing {
                name: name.to_owned(),
            });
        }
        let prior = cell_borrow_mut(&self.declarations).insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::DuplicateName {
                name: name.to_owned(),
            });
        }
        Ok(())
    }

    /// Record a variable-form head's entity.
    fn bind_variable(&self, name: &str, entity: Entity) -> Result<(), AnalyzeError> {
        if cell_borrow(&self.declarations).contains_key(name) {
            return Err(AnalyzeError::NameShadowing {
                name: name.to_owned(),
            });
        }
        let prior = cell_borrow_mut(&self.variables).insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::DuplicateName {
                name: name.to_owned(),
            });
        }
        Ok(())
    }

    /// Record an in-document `attribute!` definition for the
    /// given declaration / variable name.
    fn record_attribute(&self, name: Option<&str>, attribute: ResolvedAttribute) {
        if let Some(name) = name {
            cell_borrow_mut(&self.in_doc_attributes).insert(name.to_owned(), attribute.clone());
        }
        cell_borrow_mut(&self.in_doc_attributes_by_entity)
            .insert(attribute.entity.to_string(), attribute);
    }

    /// Record an in-document `concept!` definition.
    fn record_concept(&self, name: Option<&str>, concept: ResolvedConcept) {
        if let Some(name) = name {
            cell_borrow_mut(&self.in_doc_concepts).insert(name.to_owned(), concept.clone());
        }
        cell_borrow_mut(&self.in_doc_concepts_by_entity)
            .insert(concept.entity.to_string(), concept);
    }

    /// Look up the entity bound to a `.bookmark` or `?var` name,
    /// regardless of which side it lives on. Returns `None` if
    /// the name isn't known yet.
    fn lookup_entity(&self, name: &str) -> Option<Entity> {
        if let Some(e) = cell_borrow(&self.declarations).get(name) {
            return Some(e.clone());
        }
        cell_borrow(&self.variables).get(name).cloned()
    }

    async fn resolve_concept(&self, name: &str) -> Result<Option<ResolvedConcept>, ResolverError> {
        // Drop the borrow before awaiting the fallback resolver
        // — holding a Mutex guard across an await would deadlock
        // if the resolver came back to us.
        if let Some(found) = cell_borrow(&self.in_doc_concepts).get(name).cloned() {
            return Ok(Some(found));
        }
        self.inner.resolve_concept(name).await
    }

    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        if let Some(found) = cell_borrow(&self.in_doc_attributes).get(name).cloned() {
            return Ok(Some(found));
        }
        self.inner.resolve_attribute(name).await
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, ResolverError> {
        let key = entity.to_string();
        if let Some(found) = cell_borrow(&self.in_doc_attributes_by_entity)
            .get(&key)
            .cloned()
        {
            return Ok(Some(found));
        }
        self.inner.resolve_attribute_by_entity(entity).await
    }
}

// ---------------------------------------------------------------- //
// Public entry point                                               //
// ---------------------------------------------------------------- //

/// Analyze a parsed [`Syntax`] tree.
///
/// Three phases per `analysis-spec.md`:
/// 1. Derive `declarations` and `variables` from heads.
/// 2. Build the per-expression query [`Application`]s with
///    `.bookmark` / analysis-time `?var` substituted.
/// 3. Build the mutation [`Statement`] list with `.bookmark`
///    substituted but `?var` left for planning time.
///
/// `R: Resolver` is the public bound; on native we additionally
/// need `R: Sync` so async-trait-generated futures stay `Send`
/// (axum requires `Send` route handlers). On wasm the trait is
/// `?Send` and there's no extra bound.
#[cfg(not(target_arch = "wasm32"))]
pub async fn analyze<R: Resolver + Sync>(
    syntax: &Syntax,
    resolver: &R,
) -> Result<Analysis, AnalyzeError> {
    analyze_impl(syntax, resolver).await
}

/// wasm-side variant of [`analyze`] — same shape minus the
/// `Sync` bound (the wasm runtime is single-threaded and the
/// trait is `?Send`).
#[cfg(target_arch = "wasm32")]
pub async fn analyze<R: Resolver>(syntax: &Syntax, resolver: &R) -> Result<Analysis, AnalyzeError> {
    analyze_impl(syntax, resolver).await
}

async fn analyze_impl<R: Resolver>(
    syntax: &Syntax,
    resolver: &R,
) -> Result<Analysis, AnalyzeError> {
    if syntax.expressions.is_empty() {
        return Err(AnalyzeError::EmptyDocument);
    }

    let scope = Scope::new(resolver);

    // ---- Phase 1: derive declarations and variables ----
    //
    // For meta heads (`attribute!`, `concept!`) this also parses
    // the body so the descriptor's content-addressed entity is
    // known up front; the parsed descriptor is stashed in
    // `meta_cache` keyed by expression index so Phase 3 doesn't
    // re-do the work.
    let mut meta_cache: HashMap<usize, MetaPlan> = HashMap::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        let (head, has_effect) = match expression {
            Expression::Query(q) => (&q.head, false),
            Expression::Assertion(a) => (&a.head, true),
            Expression::Retraction(r) => (&r.head, true),
        };

        // Meta heads carry their entity in the descriptor, which
        // requires parsing the body. Do it here so the entity
        // can land in declarations/variables.
        if has_effect && let HeadName::Concept(name) = &head.name {
            match name.as_str() {
                "attribute" => {
                    let assertion = match expression {
                        Expression::Assertion(a) => a,
                        _ => continue,
                    };
                    let plan = parse_attribute_body(assertion)?;
                    let entity = plan.entity.clone();
                    let attribute = ResolvedAttribute {
                        entity: entity.clone(),
                        descriptor: plan.descriptor.clone(),
                    };
                    let binding = head_binding_for(&head.binding)?;
                    let bookmark = match &binding {
                        HeadBinding::Bookmark(name) => Some(name.clone()),
                        _ => None,
                    };
                    let variable = match &binding {
                        HeadBinding::Variable(name) => Some(name.clone()),
                        _ => None,
                    };
                    if let Some(name) = &bookmark {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
                    }
                    scope.record_attribute(bookmark.as_deref().or(variable.as_deref()), attribute);
                    meta_cache.insert(
                        index,
                        MetaPlan::Attribute {
                            descriptor: plan.descriptor,
                            entity,
                            binding,
                        },
                    );
                    continue;
                }
                "concept" => {
                    // Concept body resolution may need the
                    // resolver — defer to Phase 3 and resolve
                    // here too. The body references attributes
                    // via `.bookmark` / URIs that may live in
                    // either the in-doc map or the branch.
                    let assertion = match expression {
                        Expression::Assertion(a) => a,
                        _ => continue,
                    };
                    let plan = parse_concept_body(assertion, &scope).await?;
                    let entity = plan.entity.clone();
                    let concept = ResolvedConcept {
                        entity: entity.clone(),
                        descriptor: plan.descriptor.clone(),
                    };
                    let binding = head_binding_for(&head.binding)?;
                    let bookmark = match &binding {
                        HeadBinding::Bookmark(name) => Some(name.clone()),
                        _ => None,
                    };
                    let variable = match &binding {
                        HeadBinding::Variable(name) => Some(name.clone()),
                        _ => None,
                    };
                    if let Some(name) = &bookmark {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
                    }
                    scope.record_concept(bookmark.as_deref().or(variable.as_deref()), concept);
                    meta_cache.insert(
                        index,
                        MetaPlan::Concept {
                            descriptor: plan.descriptor,
                            entity,
                            binding,
                        },
                    );
                    continue;
                }
                _ => {}
            }
        }

        // Non-meta heads: nothing to do in Phase 1. Bookmark and
        // variable forms both produce body-content-derived
        // entities, which we don't compute until Phase 3.
        // Bookmarks act like git tags: a `dialog.meta/name`
        // claim points the bookmark name at the body-derived
        // entity. Variables stay as `Term::Variable` until
        // planning binds them from query results, or — for
        // unbound variables — Phase 3 mints a body-derived
        // entity and registers the name in `analysis.variables`.
    }

    // ---- Phase 2: build query Applications ----
    let mut analysis = Analysis {
        declarations: cell_borrow(&scope.declarations).clone(),
        variables: cell_borrow(&scope.variables).clone(),
        ..Analysis::default()
    };

    let mut queries: Vec<Application> = Vec::new();
    for expression in &syntax.expressions {
        if let Expression::Query(q) = expression {
            let application = build_query_application(q, &scope, &analysis).await?;
            queries.push(application);
        }
    }
    if !queries.is_empty() {
        analysis.query = Some(QueryAnalysis { queries });
    }

    // ---- Phase 3: build mutation Statements ----
    let mut statements: Vec<Statement> = Vec::new();
    let mut requires: HashSet<String> = HashSet::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        match expression {
            Expression::Query(_) => {}
            Expression::Assertion(a) => {
                let application =
                    build_assertion_application(index, a, &meta_cache, &scope, &mut analysis)
                        .await?;
                collect_unbound_variables(&application, &analysis, &mut requires);
                statements.push(Statement::Assert(application));
            }
            Expression::Retraction(r) => {
                let application = build_retraction_application(r, &scope, &mut analysis).await?;
                collect_unbound_variables(&application, &analysis, &mut requires);
                statements.push(Statement::Retract(application));
            }
        }
    }

    // requires must be subset of query bindings (analysis-time
    // variables already filtered out by `collect_unbound_variables`).
    if let Some(query) = &analysis.query {
        let bindings = query.bindings();
        for name in &requires {
            if !bindings.contains(name) {
                return Err(AnalyzeError::UnboundMutationVariable { name: name.clone() });
            }
        }
    } else if !requires.is_empty() {
        let name = requires.iter().next().cloned().unwrap_or_default();
        return Err(AnalyzeError::UnboundMutationVariable { name });
    }

    analysis.mutate = MutationAnalysis {
        statements,
        requires,
    };

    Ok(analysis)
}

// ---------------------------------------------------------------- //
// Phase 1 helpers — meta heads                                     //
// ---------------------------------------------------------------- //

/// Cached parse of a meta head's body — keeps Phase 3 from
/// re-parsing the body that Phase 1 already needed for the
/// content-addressed entity.
enum MetaPlan {
    Attribute {
        descriptor: AttributeDescriptor,
        entity: Entity,
        binding: HeadBinding,
    },
    Concept {
        descriptor: ConceptDescriptor,
        entity: Entity,
        binding: HeadBinding,
    },
}

/// Parsed `attribute!` body — descriptor plus entity URI.
struct AttributeBodyPlan {
    descriptor: AttributeDescriptor,
    entity: Entity,
}

fn parse_attribute_body(assertion: &Assertion) -> Result<AttributeBodyPlan, AnalyzeError> {
    let mut shape = serde_json::Map::new();
    for field in &assertion.fields {
        let value_str = match &field.value {
            FieldValue::Literal(Scalar::String(s)) => s.clone(),
            FieldValue::Literal(other) => scalar_to_string(other)?,
            FieldValue::Reference(Reference::Uri(s)) => s.clone(),
            FieldValue::Reference(Reference::Bookmark(_)) => {
                return Err(AnalyzeError::UnsupportedFieldValue {
                    field: field.name.clone(),
                    form: "bookmark reference (`attribute!` body must be literals)",
                });
            }
            FieldValue::Variable(_) | FieldValue::Blank | FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedFieldValue {
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
    Ok(AttributeBodyPlan { descriptor, entity })
}

/// Parsed `concept!` body — descriptor plus entity URI.
struct ConceptBodyPlan {
    descriptor: ConceptDescriptor,
    entity: Entity,
}

async fn parse_concept_body<R: Resolver>(
    assertion: &Assertion,
    scope: &Scope<'_, R>,
) -> Result<ConceptBodyPlan, AnalyzeError> {
    let mut description: Option<String> = None;
    let mut with_fields: Vec<(String, ResolvedAttribute)> = Vec::new();
    for field in &assertion.fields {
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
                                 attribute reference (`.bookmark` or URI)"
                            .into(),
                    });
                };
                for sub in inner {
                    let resolved = resolve_concept_field(&sub.name, &sub.value, scope).await?;
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
    Ok(ConceptBodyPlan { descriptor, entity })
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
        FieldValue::Reference(Reference::Bookmark(name)) => scope
            .resolve_attribute(name)
            .await
            .map_err(|e| AnalyzeError::ResolverFailed {
                context: format!("bookmark .{name}"),
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
            form: "expected `.bookmark` reference or `the:…` URI \
                   (bare names are literal strings — prefix with `.` \
                   to mark as a reference)",
        }),
    }
}

// ---------------------------------------------------------------- //
// Phase 2 — build query Applications                               //
// ---------------------------------------------------------------- //

async fn build_query_application<R: Resolver>(
    query: &tonk_notation::Query,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Application, AnalyzeError> {
    let binding = head_binding_for(&query.head.binding)?;
    match &query.head.name {
        HeadName::Concept(name) => {
            let resolved = scope
                .resolve_concept(name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept { name: name.clone() })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term_for_query(&binding));
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                let term = match user_field(query.fields.as_slice(), field_name) {
                    Some(value) => field_value_to_term(field_name, value, scope, analysis).await?,
                    None => Term::<dialog_query::Any>::blank(),
                };
                terms.insert(field_name.into(), term);
            }
            // Reject unknown user-supplied fields.
            for field in &query.fields {
                if resolved
                    .descriptor
                    .with()
                    .iter()
                    .all(|(n, _)| n != field.name)
                {
                    return Err(AnalyzeError::UnknownField {
                        concept: name.clone(),
                        field: field.name.clone(),
                    });
                }
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
                },
                binding,
            })
        }
        HeadName::Claim(domain) => {
            if query.fields.is_empty() {
                return Err(AnalyzeError::ClaimWithoutFields {
                    domain: domain.clone(),
                });
            }
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term_for_query(&binding));
            for field in &query.fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                },
                binding,
            })
        }
    }
}

fn user_field<'a>(fields: &'a [Field], name: &str) -> Option<&'a FieldValue> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

fn this_term_for_query(binding: &HeadBinding) -> Term<dialog_query::Any> {
    match binding {
        HeadBinding::Variable(name) => Term::<dialog_query::Any>::var(name),
        HeadBinding::Bookmark(_name) => {
            // Query-side bookmark resolution requires hitting
            // the branch to find the entity carrying
            // `dialog.meta/name = name`. The query path doesn't
            // currently do that (no async access here); use a
            // blank for now and revisit if the editor surfaces a
            // need for this shape.
            Term::<dialog_query::Any>::blank()
        }
        HeadBinding::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
        HeadBinding::Anonymous => Term::<dialog_query::Any>::blank(),
    }
}

// ---------------------------------------------------------------- //
// Phase 3 — build mutation Applications                            //
// ---------------------------------------------------------------- //

async fn build_assertion_application<R: Resolver>(
    index: usize,
    assertion: &Assertion,
    meta_cache: &HashMap<usize, MetaPlan>,
    scope: &Scope<'_, R>,
    analysis: &mut Analysis,
) -> Result<Application, AnalyzeError> {
    if let Some(meta) = meta_cache.get(&index) {
        return Ok(meta_application(meta));
    }

    let head_label = match &assertion.head.name {
        HeadName::Concept(name) => name.clone(),
        HeadName::Claim(domain) => domain.clone(),
    };

    if assertion.fields.is_empty() {
        return Err(AnalyzeError::AssertionWithoutFields { head: head_label });
    }

    let binding = head_binding_for(&assertion.head.binding)?;
    let this_term = this_term_for_assertion(&binding, &assertion.fields, analysis)?;

    match &assertion.head.name {
        HeadName::Concept(name) => {
            let resolved = scope
                .resolve_concept(name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept { name: name.clone() })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term);
            let mut user_fields: BTreeMap<&str, &FieldValue> = BTreeMap::new();
            for field in &assertion.fields {
                user_fields.insert(field.name.as_str(), &field.value);
            }
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                let Some(value) = user_fields.remove(field_name) else {
                    // Field omitted — leave a blank so the
                    // emitter skips it on assert.
                    terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
                    continue;
                };
                let term = field_value_to_term(field_name, value, scope, analysis).await?;
                terms.insert(field_name.into(), term);
            }
            if let Some((unknown, _)) = user_fields.into_iter().next() {
                return Err(AnalyzeError::UnknownField {
                    concept: name.clone(),
                    field: unknown.to_owned(),
                });
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
                },
                binding,
            })
        }
        HeadName::Claim(domain) => {
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term);
            for field in &assertion.fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                },
                binding,
            })
        }
    }
}

async fn build_retraction_application<R: Resolver>(
    retraction: &Retraction,
    scope: &Scope<'_, R>,
    analysis: &mut Analysis,
) -> Result<Application, AnalyzeError> {
    let binding = head_binding_for(&retraction.head.binding)?;
    // Retraction has no body, so the entity has to come from a
    // bookmark/variable binding (whose name resolves through
    // the branch / declarations), an explicit URI, or a query
    // binding. Anonymous retractions have no entity to act on.
    let this_term = this_term_for_retraction(&binding, analysis)?;
    match &retraction.head.name {
        HeadName::Concept(name) => {
            let resolved = scope
                .resolve_concept(name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept { name: name.clone() })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term);
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
                },
                binding,
            })
        }
        HeadName::Claim(domain) => Err(AnalyzeError::UnsupportedFieldValue {
            field: domain.clone(),
            form: "claim retraction (no descriptor to enumerate fields)",
        }),
    }
}

/// Translate a parsed [`Binding`] to the analyzer's
/// [`HeadBinding`]. Validates URIs eagerly so the `Application`
/// carries a parsed `Entity` rather than a string.
fn head_binding_for(binding: &Binding) -> Result<HeadBinding, AnalyzeError> {
    Ok(match binding {
        Binding::Anonymous => HeadBinding::Anonymous,
        Binding::Variable(name) => HeadBinding::Variable(name.clone()),
        Binding::Bookmark(name) => HeadBinding::Bookmark(name.clone()),
        Binding::Uri(uri) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeError::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            HeadBinding::Uri(entity)
        }
    })
}

fn meta_application(meta: &MetaPlan) -> Application {
    match meta {
        MetaPlan::Attribute {
            descriptor,
            entity,
            binding,
        } => {
            // Built-in `attribute` schema: 4 fields under
            // dialog.attribute/* and dialog.meta/description.
            // The bookmark name (`dialog.meta/name` claim) is
            // emitted by the planner via `HeadBinding::Bookmark`,
            // not encoded as a parameter — same way it works for
            // any user concept's bookmark binding.
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
            // `AnonymousAttribute`/`NamedAttribute` require all
            // five claims to be present (id/type/cardinality/
            // description/name) — ConceptByEntity reconstruction
            // depends on the full set. Emit every field with an
            // empty-string default so `dialog.attribute/type` and
            // `dialog.meta/description` always exist.
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
                binding: binding.clone(),
            }
        }
        MetaPlan::Concept {
            descriptor,
            entity,
            binding,
        } => {
            // Built-in `concept` schema: one `with.<field>` per
            // field of the user's concept, plus
            // `dialog.meta/description`. The bookmark name is
            // emitted by the planner via `HeadBinding::Bookmark`.
            let mut terms = Parameters::new();
            terms.insert("this".into(), Term::Constant(Value::Entity(entity.clone())));
            for (name, attr) in descriptor.with().iter() {
                let attr_entity: Entity = attr
                    .to_uri()
                    .parse()
                    .expect("AttributeDescriptor::to_uri produces a valid entity");
                terms.insert(
                    format!("with.{name}"),
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
                binding: binding.clone(),
            }
        }
    }
}

/// Build the `dialog.attribute` built-in schema descriptor. Its
/// fields map to the 5 EAVs every named attribute writes. Kept
/// in sync with `meta::AnonymousAttribute` / `NamedAttribute`.
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
/// field of the concept being defined, plus optional name and
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

/// What to put in the `this` slot of a mutation [`Application`].
///
/// - `Anonymous`: mint a body-content-derived entity.
/// - `Bookmark(name)`: same — body-derived entity. The
///   `dialog.meta/name = name` claim is emitted by the planner
///   from the [`HeadBinding::Bookmark`] discriminant, not a
///   parameter on the predicate.
/// - `Variable(name)` already in `analysis.variables`: substitute
///   the registered entity (this happens when an earlier
///   `?name:` head registered it, or when an `attribute! ?name:`
///   / `concept! ?name:` head derived it from the body).
/// - `Variable(name)` not yet known: if there's no query
///   binding for it, mint a body-derived entity and register it
///   in `analysis.variables` so subsequent uses share the
///   entity. If a query binding exists, leave as
///   `Term::Variable` — planning will substitute from the
///   query frame.
/// - `Uri(entity)`: substitute directly.
fn this_term_for_assertion(
    binding: &HeadBinding,
    fields: &[Field],
    analysis: &mut Analysis,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match binding {
        HeadBinding::Anonymous => Term::Constant(Value::Entity(Entity::of(&body_digest(fields)))),
        HeadBinding::Bookmark(name) => {
            // Non-meta bookmark: body-derived entity. Register
            // the name → entity binding in `declarations` so
            // the worker can surface it in the response and
            // duplicate-name checks across heads catch overlaps.
            let entity = Entity::of(&body_digest(fields));
            if let Some(prior) = analysis.declarations.get(name)
                && prior != &entity
            {
                return Err(AnalyzeError::DuplicateName { name: name.clone() });
            }
            analysis.declarations.insert(name.clone(), entity.clone());
            Term::Constant(Value::Entity(entity))
        }
        HeadBinding::Variable(name) => {
            if let Some(entity) = analysis.variables.get(name) {
                Term::Constant(Value::Entity(entity.clone()))
            } else if query_binds(analysis, name) {
                // Bound at planning time from query results.
                Term::<dialog_query::Any>::var(name)
            } else {
                // First introduction — mint a body-derived
                // entity and register it for later expressions
                // that share `?name`.
                let entity = Entity::of(&body_digest(fields));
                analysis.variables.insert(name.clone(), entity.clone());
                Term::Constant(Value::Entity(entity))
            }
        }
        HeadBinding::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
    })
}

/// `this` term for a retraction (no body to hash). The entity
/// must come from a Bookmark (looked up by name on the branch),
/// a Variable (registered earlier or query-bound), or a URI.
/// `Anonymous` retraction has no entity to act on.
fn this_term_for_retraction(
    binding: &HeadBinding,
    analysis: &Analysis,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match binding {
        HeadBinding::Anonymous => {
            return Err(AnalyzeError::UnsupportedFieldValue {
                field: "head".into(),
                form: "anonymous retraction has no entity to act on; \
                       bind the head with `?var`, a bookmark, or a `did:key:…` URI",
            });
        }
        HeadBinding::Bookmark(_name) => {
            // We can't synchronously resolve the bookmark to an
            // entity here without a branch query; the worker's
            // retraction path would have to take a name and run
            // the lookup. For now: leave as a blank — the worker
            // will need to handle this case explicitly when we
            // wire it. Most retractions are URI-bound.
            Term::<dialog_query::Any>::blank()
        }
        HeadBinding::Variable(name) => match analysis.variables.get(name) {
            Some(entity) => Term::Constant(Value::Entity(entity.clone())),
            None => Term::<dialog_query::Any>::var(name),
        },
        HeadBinding::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
    })
}

/// Hash-stable summary of an assertion body — pairs of
/// `(field_name, FieldDigest)` sorted by name. Used by
/// `Entity::of` to derive a content-addressed entity for
/// `Anonymous` / `Bookmark` / unbound `Variable` heads.
///
/// Only literal scalars contribute. Variables, references, and
/// blanks are skipped — they're not part of the entity's
/// identity (they'd reference *other* entities, and including
/// them in the hash would defeat the deterministic-rerun
/// property).
fn body_digest(fields: &[Field]) -> Vec<(String, FieldDigest)> {
    let mut out: Vec<(String, FieldDigest)> = Vec::new();
    for field in fields {
        let digest = match &field.value {
            FieldValue::Literal(scalar) => FieldDigest::from_scalar(scalar),
            // Skip variables, references, blanks, nested.
            _ => continue,
        };
        out.push((field.name.clone(), digest));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Serializable shadow of [`Scalar`] used only by
/// [`body_digest`]. Round-trips the scalar's primitive value so
/// `Entity::of` can hash it deterministically. Distinct from
/// `Scalar` because we want a stable serde representation
/// independent of any future surface-syntax changes.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum FieldDigest {
    String(String),
    Integer(i128),
    UnsignedInteger(u128),
    Float(f64),
    Boolean(bool),
    Null,
}

impl FieldDigest {
    fn from_scalar(scalar: &Scalar) -> Self {
        match scalar {
            Scalar::String(s) => Self::String(s.clone()),
            Scalar::Integer(i) => Self::Integer(*i),
            Scalar::UnsignedInteger(u) => Self::UnsignedInteger(*u),
            Scalar::Float(f) => Self::Float(*f),
            Scalar::Boolean(b) => Self::Boolean(*b),
            Scalar::Null => Self::Null,
        }
    }
}

/// Does `analysis.query` bind `?name`? Used by
/// [`this_term_for_assertion`] to decide between minting a
/// body-derived entity and leaving the variable for planning.
fn query_binds(analysis: &Analysis, name: &str) -> bool {
    analysis
        .query
        .as_ref()
        .map(|q| q.bindings().contains(name))
        .unwrap_or(false)
}

// ---------------------------------------------------------------- //
// Field-value substitution (`.bookmark`, `?var`, literals)         //
// ---------------------------------------------------------------- //

/// Translate a parsed [`FieldValue`] into the `Term<Any>` slot it
/// belongs in. Bookmarks resolve at analysis time (against in-doc
/// declarations first, then the branch); variables resolve
/// against `analysis.variables` if known, otherwise stay as
/// `Term::Variable` so planning can substitute them later;
/// literals become `Term::Constant`; blanks become
/// `Term::blank()`.
async fn field_value_to_term<R: Resolver>(
    field_name: &str,
    value: &FieldValue,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match value {
        FieldValue::Literal(scalar) => Term::Constant(scalar_to_value(scalar)?),
        FieldValue::Variable(name) => {
            // If this variable was derived in Phase 1, substitute
            // the entity now; otherwise leave it as a variable
            // that planning will bind from query results.
            if let Some(entity) = analysis.variables.get(name) {
                Term::Constant(Value::Entity(entity.clone()))
            } else {
                Term::<dialog_query::Any>::var(name)
            }
        }
        FieldValue::Reference(Reference::Bookmark(name)) => {
            // Look up declarations first (the head bound it),
            // then in-doc attributes (an `attribute!` defined
            // earlier in this document), then the branch.
            if let Some(entity) = scope.lookup_entity(name) {
                Term::Constant(Value::Entity(entity))
            } else if let Some(resolved) =
                scope
                    .resolve_attribute(name)
                    .await
                    .map_err(|e| AnalyzeError::ResolverFailed {
                        context: format!("bookmark .{name}"),
                        reason: e.message,
                    })?
            {
                Term::Constant(Value::Entity(resolved.entity))
            } else {
                return Err(AnalyzeError::UnknownBookmark {
                    field: field_name.into(),
                    bookmark: name.clone(),
                });
            }
        }
        FieldValue::Reference(Reference::Uri(uri)) => {
            let entity: Entity =
                uri.parse()
                    .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                        AnalyzeError::InvalidSubjectUri {
                            subject: uri.clone(),
                            reason: e.to_string(),
                        }
                    })?;
            Term::Constant(Value::Entity(entity))
        }
        FieldValue::Blank => Term::<dialog_query::Any>::blank(),
        FieldValue::Nested(_) => {
            return Err(AnalyzeError::UnsupportedFieldValue {
                field: field_name.into(),
                form: "nested mapping (only `concept!`'s `with:` accepts a nested map)",
            });
        }
    })
}

fn scalar_to_value(scalar: &Scalar) -> Result<Value, AnalyzeError> {
    Ok(match scalar {
        Scalar::String(s) => Value::String(s.clone()),
        Scalar::Boolean(b) => Value::Boolean(*b),
        Scalar::Integer(i) => Value::SignedInt(*i),
        Scalar::UnsignedInteger(u) => Value::UnsignedInt(*u),
        Scalar::Float(f) => Value::Float(*f),
        Scalar::Null => {
            return Err(AnalyzeError::UnsupportedFieldValue {
                field: "<scalar>".into(),
                form: "null literal",
            });
        }
    })
}

fn scalar_to_string(scalar: &Scalar) -> Result<String, AnalyzeError> {
    Ok(match scalar {
        Scalar::String(s) => s.clone(),
        Scalar::Boolean(b) => b.to_string(),
        Scalar::Integer(i) => i.to_string(),
        Scalar::UnsignedInteger(u) => u.to_string(),
        Scalar::Float(f) => f.to_string(),
        Scalar::Null => {
            return Err(AnalyzeError::UnsupportedFieldValue {
                field: "<scalar>".into(),
                form: "null literal",
            });
        }
    })
}

fn validate_claim_attribute(domain: &str, field: &str) -> Result<(), AnalyzeError> {
    let uri = format!("{domain}/{field}");
    uri.parse::<AttributeThe>()
        .map(|_| ())
        .map_err(|e| AnalyzeError::InvalidClaimAttribute {
            domain: domain.to_owned(),
            field: field.to_owned(),
            reason: format!("{e}"),
        })
}

fn collect_unbound_variables(
    application: &Application,
    analysis: &Analysis,
    out: &mut HashSet<String>,
) {
    for name in application.bindings() {
        if !analysis.variables.contains_key(&name) {
            out.insert(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transact::Application;
    use tonk_notation::parse;

    /// `parse` returns `Parsed`; tests want the `Syntax` and panic on diagnostics.
    fn must_parse(src: &str) -> Syntax {
        let parsed = parse(src);
        assert!(
            parsed.diagnostics.is_empty(),
            "parse diagnostics: {:#?}",
            parsed.diagnostics
        );
        parsed
            .syntax
            .expect("parser produces a Syntax for non-empty input")
    }

    /// Resolver that hands back a fixed `(name → ConceptDescriptor)`
    /// and errors on attribute lookups.
    struct FixedConcept {
        name: String,
        descriptor: ConceptDescriptor,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Resolver for FixedConcept {
        async fn resolve_concept(
            &self,
            name: &str,
        ) -> Result<Option<ResolvedConcept>, ResolverError> {
            if name == self.name {
                Ok(Some(ResolvedConcept {
                    entity: self.descriptor.this(),
                    descriptor: self.descriptor.clone(),
                }))
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

    fn fixed_concept(name: &str, fields: &[(&str, &str)]) -> FixedConcept {
        let mut with = serde_json::Map::new();
        for (field, the) in fields {
            with.insert(
                (*field).into(),
                serde_json::json!({ "the": the, "as": "Text", "cardinality": "one" }),
            );
        }
        let descriptor: ConceptDescriptor =
            serde_json::from_value(serde_json::json!({ "with": with })).unwrap();
        FixedConcept {
            name: name.into(),
            descriptor,
        }
    }

    #[dialog_common::test]
    async fn empty_document_is_an_error() {
        let syntax = Syntax {
            expressions: Vec::new(),
            range: lsp_types::Range::default(),
        };
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::EmptyDocument));
    }

    /// `attribute! foo:` declares a content-derived attribute
    /// entity in `declarations`, an Assert statement, no query,
    /// no requires.
    #[dialog_common::test]
    async fn single_attribute_assertion() {
        let syntax = must_parse(
            "attribute! person-name:\n\
             \x20 the:         io.gozala.person/name\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n",
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.variables.is_empty());
        assert!(analysis.query.is_none());
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(analysis.mutate.requires.is_empty());
        let Statement::Assert(Application::Concept { .. }) = &analysis.mutate.statements[0] else {
            panic!("expected Assert(Concept)");
        };
    }

    /// Three-expression doc: two `attribute!` + one `concept!`
    /// referencing them via `.bookmark`. Concept body resolution
    /// must hit the in-doc index, not the (Noop) outer resolver.
    #[dialog_common::test]
    async fn attributes_then_concept_in_one_doc() {
        let syntax = must_parse(
            "attribute! person-name:\n\
             \x20 the:         io.gozala.person/name\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n\
             attribute! person-age:\n\
             \x20 the:         io.gozala.person/age\n\
             \x20 as:          UnsignedInteger\n\
             \x20 cardinality: one\n\
             concept! person:\n\
             \x20 with:\n\
             \x20   name: .person-name\n\
             \x20   age:  .person-age\n",
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.declarations.contains_key("person-age"));
        assert!(analysis.declarations.contains_key("person"));
        assert!(analysis.query.is_none());
        // 3 statements — 2 attribute + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
    }

    /// Variable-form `attribute! ?foo:` lands in `variables`,
    /// not `declarations`, and does NOT emit a `dialog.meta/name`
    /// claim (the name is doc-scoped only).
    #[dialog_common::test]
    async fn variable_form_attribute_is_doc_scoped() {
        let syntax = must_parse(
            "attribute! ?person-name:\n\
             \x20 the:         io.gozala.person/name\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n",
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.is_empty());
        assert!(analysis.variables.contains_key("person-name"));
        let Statement::Assert(Application::Concept { query, binding }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        // No `name` term — meta-name is carried by the binding,
        // not as a parameter.
        assert!(query.terms.get("name").is_none());
        assert!(matches!(binding, HeadBinding::Variable(s) if s == "person-name"));
    }

    /// `attribute! foo:` (bookmark form): the head's `binding`
    /// records the bookmark string. The planner emits the
    /// `dialog.meta/name` claim from `HeadBinding::Bookmark`,
    /// not from a parameter.
    #[dialog_common::test]
    async fn bookmark_form_attribute_carries_binding() {
        let syntax = must_parse(
            "attribute! person-name:\n\
             \x20 the:         io.gozala.person/name\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n",
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, binding }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(query.terms.get("name").is_none());
        assert!(matches!(binding, HeadBinding::Bookmark(s) if s == "person-name"));
    }

    /// Two meta heads of different kinds (`attribute!` and
    /// `concept!`) that share a bookmark name → `DuplicateName`.
    /// Phase 1 sees both heads declare `foo` and the second
    /// `declare` returns `Some(prior_entity)`, triggering the
    /// error. Only meta heads register declarations in Phase 1
    /// — non-meta heads defer their entity to Phase 3 and so
    /// are not checked for name collisions today.
    #[dialog_common::test]
    async fn duplicate_meta_bookmarks_is_an_error() {
        // The concept's `with: { x: .a }` references the `a`
        // attribute defined just above, so concept-body
        // resolution succeeds and Phase 1 reaches the second
        // `declare("a", …)` call which finds the prior entry.
        let syntax = must_parse(
            "attribute! a:\n\
             \x20 the:         x.y/a\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n\
             concept! a:\n\
             \x20 with:\n\
             \x20   x: .a\n",
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(err, AnalyzeError::DuplicateName { .. }),
            "expected DuplicateName, got {err:?}"
        );
    }

    /// A bookmark and a variable with the same name → `NameShadowing`.
    #[dialog_common::test]
    async fn bookmark_and_variable_same_name_is_an_error() {
        let syntax = must_parse(
            "attribute! foo:\n\
             \x20 the:         x.y/a\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n\
             attribute! ?foo:\n\
             \x20 the:         x.y/b\n\
             \x20 as:          Text\n\
             \x20 cardinality: one\n",
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::NameShadowing { .. }));
    }

    /// Pure-query document: `Analysis::query` is `Some`, no
    /// statements, no requires.
    #[dialog_common::test]
    async fn pure_query_document() {
        let syntax = must_parse("person ?alice:\n  name: \"Alice\"\n");
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(analysis.query.is_some());
        assert!(analysis.mutate.statements.is_empty());
        assert!(analysis.mutate.requires.is_empty());
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        assert!(q.bindings().contains("alice"));
    }

    /// Query + assertion joined by `?alice`: the mutation
    /// records `alice` in `requires`, and the analyzer does
    /// not error (the query binds it).
    #[dialog_common::test]
    async fn query_then_assert_joined_by_variable() {
        let syntax = must_parse(
            "person ?alice:\n\
             \x20 name: \"Alice\"\n\
             person! ?alice:\n\
             \x20 name: \"Renamed\"\n",
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert!(analysis.query.is_some());
        assert_eq!(analysis.mutate.statements.len(), 1);
        assert!(analysis.mutate.requires.contains("alice"));
    }

    /// Mutation references `?bogus` that no source binds →
    /// `UnboundMutationVariable`.
    #[dialog_common::test]
    async fn unbound_mutation_variable_is_an_error() {
        let syntax = must_parse(
            "person! ?ghost:\n\
             \x20 name: ?nope\n",
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnboundMutationVariable { .. }));
    }

    /// Concept retraction: blank terms for every field, the
    /// `this` term carries the bookmark-derived entity.
    #[dialog_common::test]
    async fn concept_retraction_blanks_every_field() {
        let syntax =
            must_parse("person! did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv: _\n");
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Retract(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Retract(Concept)");
        };
        // `this` is bound; all other fields are blank.
        assert!(matches!(
            q.terms.get("this"),
            Some(Term::Constant(Value::Entity(_)))
        ));
        for field in ["name", "age"] {
            assert!(
                matches!(q.terms.get(field), Some(Term::Variable { name: None, .. })),
                "field {field:?} should be blank"
            );
        }
    }

    /// Unknown concept in head position → `UnknownConcept`.
    #[dialog_common::test]
    async fn unknown_concept_errors() {
        let syntax = must_parse("nope:\n  field: \"x\"\n");
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnknownConcept { .. }));
    }

    /// Claim head with no fields → `ClaimWithoutFields`.
    #[dialog_common::test]
    async fn claim_without_fields_errors() {
        let syntax = must_parse("xyz.tonk:\n");
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::ClaimWithoutFields { .. }));
    }

    /// Claim heads build a synthesized predicate with one
    /// `<domain>/<field>` attribute per parameter.
    #[dialog_common::test]
    async fn claim_head_synthesizes_descriptor() {
        let syntax = must_parse("xyz.tonk:\n  role: ?role\n  contact: \"alice\"\n");
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let Application::Domain { application: d, .. } = &q.queries[0] else {
            panic!("expected Domain application");
        };
        assert_eq!(d.domain, "xyz.tonk");
        assert!(d.parameters.contains("role"));
        assert!(d.parameters.contains("contact"));
    }
}
