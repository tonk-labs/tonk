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
    Analysis, Application, DomainApplication, MutationAnalysis, QueryAnalysis, Statement,
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
                    let bookmark = match &head.binding {
                        Binding::Bookmark(name) => Some(name.clone()),
                        _ => None,
                    };
                    let variable = match &head.binding {
                        Binding::Variable(name) => Some(name.clone()),
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
                            bookmark,
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
                    let bookmark = match &head.binding {
                        Binding::Bookmark(name) => Some(name.clone()),
                        _ => None,
                    };
                    let variable = match &head.binding {
                        Binding::Variable(name) => Some(name.clone()),
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
                            bookmark,
                        },
                    );
                    continue;
                }
                _ => {}
            }
        }

        // Non-meta heads: derive the entity from the binding
        // form. Bookmarks are content-addressed from the name;
        // variables get a fresh random entity (so different
        // documents don't collide on the same `?alice`).
        match &head.binding {
            Binding::Bookmark(name) => {
                scope.declare(name, Entity::of(name))?;
            }
            Binding::Variable(name) => {
                let entity = Entity::new().map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("minting variable entity for ?{name}"),
                    reason: e.to_string(),
                })?;
                scope.bind_variable(name, entity)?;
            }
            Binding::Anonymous | Binding::Uri(_) => {}
        }
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
                    build_assertion_application(index, a, &meta_cache, &scope, &analysis).await?;
                collect_unbound_variables(&application, &analysis, &mut requires);
                statements.push(Statement::Assert(application));
            }
            Expression::Retraction(r) => {
                let application = build_retraction_application(r, &scope, &analysis).await?;
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
        bookmark: Option<String>,
    },
    Concept {
        descriptor: ConceptDescriptor,
        entity: Entity,
        bookmark: Option<String>,
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
            terms.insert("this".into(), this_term_for_query(&query.head.binding));
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
            Ok(Application::Concept(ConceptQuery {
                terms,
                predicate: resolved.descriptor,
            }))
        }
        HeadName::Claim(domain) => {
            if query.fields.is_empty() {
                return Err(AnalyzeError::ClaimWithoutFields {
                    domain: domain.clone(),
                });
            }
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term_for_query(&query.head.binding));
            for field in &query.fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain(DomainApplication {
                domain: domain.clone(),
                parameters,
            }))
        }
    }
}

fn user_field<'a>(fields: &'a [Field], name: &str) -> Option<&'a FieldValue> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

fn this_term_for_query(binding: &Binding) -> Term<dialog_query::Any> {
    match binding {
        Binding::Variable(name) => Term::<dialog_query::Any>::var(name),
        Binding::Bookmark(name) => Term::Constant(Value::Entity(Entity::of(name))),
        Binding::Uri(uri) => match uri.parse::<Entity>() {
            Ok(entity) => Term::Constant(Value::Entity(entity)),
            Err(_) => Term::<dialog_query::Any>::blank(),
        },
        Binding::Anonymous => Term::<dialog_query::Any>::blank(),
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
    analysis: &Analysis,
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

    let this_entity = entity_for_assertion(&assertion.head.binding)?;

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
            terms.insert("this".into(), Term::Constant(Value::Entity(this_entity)));
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
            // Bookmark binding writes a `dialog.meta/name` claim.
            if let Binding::Bookmark(name) = &assertion.head.binding {
                terms.insert("name".into(), Term::Constant(Value::String(name.clone())));
            }
            if let Some((unknown, _)) = user_fields.into_iter().next() {
                return Err(AnalyzeError::UnknownField {
                    concept: name.clone(),
                    field: unknown.to_owned(),
                });
            }
            // For bookmark binding we need a name attribute on
            // the descriptor too — extend the descriptor in-flight
            // by re-deriving it with a name field.
            let descriptor = if matches!(&assertion.head.binding, Binding::Bookmark(_)) {
                augment_descriptor_with_name(&resolved.descriptor)?
            } else {
                resolved.descriptor
            };
            Ok(Application::Concept(ConceptQuery {
                terms,
                predicate: descriptor,
            }))
        }
        HeadName::Claim(domain) => {
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), Term::Constant(Value::Entity(this_entity)));
            for field in &assertion.fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain(DomainApplication {
                domain: domain.clone(),
                parameters,
            }))
        }
    }
}

async fn build_retraction_application<R: Resolver>(
    retraction: &Retraction,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Application, AnalyzeError> {
    let _ = analysis;
    let this_entity = entity_for_assertion(&retraction.head.binding)?;
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
            terms.insert("this".into(), Term::Constant(Value::Entity(this_entity)));
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                terms.insert(field_name.into(), Term::<dialog_query::Any>::blank());
            }
            Ok(Application::Concept(ConceptQuery {
                terms,
                predicate: resolved.descriptor,
            }))
        }
        HeadName::Claim(domain) => Err(AnalyzeError::UnsupportedFieldValue {
            field: domain.clone(),
            form: "claim retraction (no descriptor to enumerate fields)",
        }),
    }
}

fn meta_application(meta: &MetaPlan) -> Application {
    match meta {
        MetaPlan::Attribute {
            descriptor,
            entity,
            bookmark,
        } => {
            // Built-in `attribute` schema: 5 fields under
            // dialog.attribute/* and dialog.meta/*. Build a
            // ConceptDescriptor whose `with` map enumerates them
            // so emit_predicate_facts writes the canonical EAVs.
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
            if let Some(ty) = descriptor.content_type() {
                let type_name = serde_json::to_value(ty)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                if !type_name.is_empty() {
                    terms.insert("type".into(), Term::Constant(Value::String(type_name)));
                }
            }
            let cardinality_name = serde_json::to_value(descriptor.cardinality())
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "one".into());
            terms.insert(
                "cardinality".into(),
                Term::Constant(Value::String(cardinality_name)),
            );
            let description = descriptor.description().to_owned();
            if !description.is_empty() {
                terms.insert(
                    "description".into(),
                    Term::Constant(Value::String(description)),
                );
            }
            if let Some(name) = bookmark {
                terms.insert("name".into(), Term::Constant(Value::String(name.clone())));
            }
            Application::Concept(ConceptQuery {
                terms,
                predicate: attribute_schema(),
            })
        }
        MetaPlan::Concept {
            descriptor,
            entity,
            bookmark,
        } => {
            // Built-in `concept` schema: one `with.<field>` per
            // field of the user's concept, plus `dialog.meta/name`
            // and `dialog.meta/description`.
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
            if let Some(name) = bookmark {
                terms.insert("name".into(), Term::Constant(Value::String(name.clone())));
            }
            Application::Concept(ConceptQuery {
                terms,
                predicate: concept_schema(descriptor),
            })
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

/// Add a `name` field (mapping to `dialog.meta/name`) to a
/// resolved concept's descriptor so a bookmark-bound assertion
/// can include the name claim alongside the user's fields.
fn augment_descriptor_with_name(
    descriptor: &ConceptDescriptor,
) -> Result<ConceptDescriptor, AnalyzeError> {
    let mut shape = serde_json::to_value(descriptor)
        .map_err(|e| AnalyzeError::InvalidConceptBody {
            reason: format!("could not re-serialize descriptor: {e}"),
        })?
        .as_object()
        .cloned()
        .unwrap_or_default();
    let with = shape
        .entry("with".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(map) = with {
        map.insert(
            "name".into(),
            serde_json::json!({
                "the": "dialog.meta/name",
                "as": "Text",
                "cardinality": "one",
            }),
        );
    }
    serde_json::from_value(serde_json::Value::Object(shape)).map_err(|e| {
        AnalyzeError::InvalidConceptBody {
            reason: format!("could not augment descriptor with name: {e}"),
        }
    })
}

fn entity_for_assertion(binding: &Binding) -> Result<Entity, AnalyzeError> {
    Ok(match binding {
        Binding::Anonymous => Entity::new().map_err(|e| AnalyzeError::ResolverFailed {
            context: "minting fresh entity".into(),
            reason: e.to_string(),
        })?,
        Binding::Variable(name) => Entity::new().map_err(|e| AnalyzeError::ResolverFailed {
            context: format!("minting fresh entity for variable ?{name}"),
            reason: e.to_string(),
        })?,
        Binding::Bookmark(name) => Entity::of(name),
        Binding::Uri(uri) => uri
            .parse()
            .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                AnalyzeError::InvalidSubjectUri {
                    subject: uri.clone(),
                    reason: e.to_string(),
                }
            })?,
    })
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
    // Tests rewritten alongside the new analyzer in a follow-up
    // (the old test fixtures referenced types that no longer
    // exist). The implementation is exercised end-to-end by the
    // worker's `evaluate` route tests.
}
