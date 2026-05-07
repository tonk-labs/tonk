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
use dialog_common::ConditionalSync;
use dialog_query::{
    AttributeDescriptor, ConceptDescriptor, Parameters, Term, attribute::The as AttributeThe,
    concept::query::ConceptQuery,
};
use thiserror::Error;
use tonk_notation::{Anchor, Assertion, Expression, Field, FieldValue, HeadName, Scalar, Syntax};

use crate::prelude::EntityExt;
use crate::transact::{
    Analysis, Application, DomainApplication, MutationAnalysis, QueryAnalysis, Statement,
    ThisIntent,
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

    /// Resolve any entity by its `dialog.meta/name` claim.
    /// Powers `.bookmark` references that point at concepts or
    /// concept instances rather than attributes — `view!`
    /// referencing `.person` should pick up a `concept! person:`
    /// definition or a `person! foo:` instance with `name: foo`.
    /// Returns the entity URI; the analyzer doesn't need a
    /// descriptor here because the bookmark only uses the entity
    /// as a constant in field-value position.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolverError>;
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
    async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolverError> {
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
        if let Some(found) = crate::builtin::lookup_concept(name) {
            return Ok(Some(found));
        }
        self.inner.resolve_concept(name).await
    }

    /// Resolve a `.bookmark` to *any* in-doc or branch entity
    /// with that name. Used by [`field_value_to_term`] when the
    /// bookmark doesn't match an attribute — concepts and
    /// previously-asserted instances also have
    /// `dialog.meta/name` claims and should be reachable.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolverError> {
        if let Some(found) = cell_borrow(&self.in_doc_concepts).get(name).cloned() {
            return Ok(Some(found.entity));
        }
        self.inner.resolve_named_entity(name).await
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
/// `R: Resolver + ConditionalSync` works on both native and
/// wasm: [`ConditionalSync`] expands to `Send + Sync` on native
/// (so async-trait-generated futures stay `Send` for axum
/// handlers) and to nothing on wasm (single-threaded runtime,
/// `Resolver` itself is `?Send` there).
pub async fn analyze<R: Resolver + ConditionalSync>(
    syntax: &Syntax,
    resolver: &R,
) -> Result<Analysis, AnalyzeError> {
    if syntax.expressions.is_empty() {
        return Err(AnalyzeError::EmptyDocument);
    }

    let scope = Scope::new(resolver);

    // ---- Phase 1: derive declarations and variables ----
    //
    // For `attribute!` / `concept!` heads this also parses the
    // body so the descriptor's content-addressed entity is known
    // up front, then builds the `Application` on the spot. The
    // built application is stashed by source-expression index
    // so Phase 3 just emits it.
    let mut declared: HashMap<usize, DeclaredApplication> = HashMap::new();

    for (index, expression) in syntax.expressions.iter().enumerate() {
        let (head, has_effect) = match expression {
            Expression::Query(q) => (&q.head, false),
            Expression::Assertion(a) => (&a.head, true),
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
                    let (this, name) =
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref())?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
                    }
                    scope.record_attribute(name.as_deref().or(variable.as_deref()), attribute);
                    let application = attribute_application(&plan.descriptor, &entity, name);
                    declared.insert(
                        index,
                        DeclaredApplication {
                            application,
                            inline_attributes: Vec::new(),
                        },
                    );
                    continue;
                }
                "concept" => {
                    // Concept body resolution may need the
                    // resolver — defer to Phase 3 and resolve
                    // here too. The body references attributes
                    // via bare-symbol / URIs that may live in
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
                    let (this, name) =
                        derive_head_intent(&assertion.fields, assertion.anchor.as_ref())?;
                    let variable = match &this {
                        ThisIntent::Variable(v) => Some(v.clone()),
                        _ => None,
                    };
                    if let Some(name) = &name {
                        scope.declare(name, entity.clone())?;
                    }
                    if let Some(name) = &variable {
                        scope.bind_variable(name, entity.clone())?;
                    }
                    scope.record_concept(name.as_deref().or(variable.as_deref()), concept);
                    // Inline attrs declared inside the concept's
                    // `with:` map publish no name — the concept's
                    // local field key is not a global label for
                    // the attribute entity.
                    let inline_attributes: Vec<Application> = plan
                        .inline_attributes
                        .into_iter()
                        .map(|attr| attribute_application(&attr.descriptor, &attr.entity, None))
                        .collect();
                    let application = concept_application(&plan.descriptor, &entity, name);
                    declared.insert(
                        index,
                        DeclaredApplication {
                            application,
                            inline_attributes,
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
                if let Some(declaration) = declared.remove(&index) {
                    // `attribute!` / `concept!` head — Phase 1
                    // already built the application. Inline
                    // attribute definitions inside a `concept!`'s
                    // `with:` map are emitted as their own
                    // assertions *before* the concept itself, so
                    // the attribute facts are present on the
                    // branch (queryable via `attribute:`) by the
                    // time anything reads back.
                    for inline in declaration.inline_attributes {
                        collect_unbound_variables(&inline, &analysis, &mut requires);
                        statements.push(Statement::Assert(inline));
                    }
                    collect_unbound_variables(&declaration.application, &analysis, &mut requires);
                    statements.push(Statement::Assert(declaration.application));
                } else {
                    let application = build_assertion_application(a, &scope, &mut analysis).await?;
                    collect_unbound_variables(&application, &analysis, &mut requires);
                    statements.push(Statement::Assert(application));
                }
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
struct DeclaredApplication {
    /// The head's own `Application`, ready to commit.
    application: Application,
    /// Anonymous attribute applications declared inline inside
    /// this concept's `with:` map. Empty for `attribute!` heads.
    inline_attributes: Vec<Application>,
}

/// Parsed `attribute!` body — descriptor plus entity URI.
struct AttributeBodyPlan {
    descriptor: AttributeDescriptor,
    entity: Entity,
}

fn parse_attribute_body(assertion: &Assertion) -> Result<AttributeBodyPlan, AnalyzeError> {
    parse_attribute_fields(&assertion.fields)
}

/// Parse an attribute definition's fields into a descriptor.
///
/// Used by `attribute! …:` heads (Phase 1, via [`parse_attribute_body`])
/// and by inline `with: { foo: { the: …, as: …, … } }` definitions
/// nested inside a `concept!` body. Same shape: `the`, `as`,
/// `cardinality`, and a *required* `description`.
fn parse_attribute_fields(fields: &[Field]) -> Result<AttributeBodyPlan, AnalyzeError> {
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
    Ok(AttributeBodyPlan { descriptor, entity })
}

/// Parsed `concept!` body — descriptor plus entity URI plus any
/// inline attribute definitions that need to be registered as
/// their own meta-head plans alongside the concept's own.
struct ConceptBodyPlan {
    descriptor: ConceptDescriptor,
    entity: Entity,
    /// Attributes defined inline in the `with:` map (as opposed to
    /// referenced via `.bookmark` / URI). Each carries the
    /// descriptor needed to emit `dialog.attribute/{id,type,
    /// cardinality}` + `dialog.meta/description` claims so the
    /// attribute is queryable via `attribute:` after the
    /// `concept!` commits.
    inline_attributes: Vec<AttributeBodyPlan>,
}

async fn parse_concept_body<R: Resolver>(
    assertion: &Assertion,
    scope: &Scope<'_, R>,
) -> Result<ConceptBodyPlan, AnalyzeError> {
    let mut description: Option<String> = None;
    let mut with_fields: Vec<(String, ResolvedAttribute)> = Vec::new();
    let mut inline_attributes: Vec<AttributeBodyPlan> = Vec::new();
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
                                 attribute reference (`.bookmark`, `?var`, \
                                 URI) or inline attribute definition \
                                 (mapping with `the`/`as`/`cardinality`/\
                                 `description`)"
                            .into(),
                    });
                };
                for sub in inner {
                    if let FieldValue::Nested(attr_fields) = &sub.value {
                        // Inline attribute definition. Parse it as
                        // an attribute body and register it for
                        // emission as a separate meta-head plan.
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
    Ok(ConceptBodyPlan {
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

// ---------------------------------------------------------------- //
// Phase 2 — build query Applications                               //
// ---------------------------------------------------------------- //

async fn build_query_application<R: Resolver>(
    query: &tonk_notation::Query,
    scope: &Scope<'_, R>,
    analysis: &Analysis,
) -> Result<Application, AnalyzeError> {
    // Queries can't carry an `&anchor` (parser rejects that), so
    // intent derivation only inspects `this:`. The returned name
    // is always `None` here.
    let (this, _name) = derive_head_intent(&query.fields, None)?;
    match &query.head.name {
        HeadName::Concept(concept_name) => {
            let resolved = scope
                .resolve_concept(concept_name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {concept_name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept {
                    name: concept_name.clone(),
                })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term_for_query(&this));
            for (field_name, _attr) in resolved.descriptor.with().iter() {
                // Fields the user mentioned use whatever they
                // wrote (literal, variable, blank, etc.). Fields
                // they *omitted* default to a named variable so
                // matches surface the value in the response —
                // `person:` reads the same as
                // `person:\n  name: ?name\n  age: ?age`.
                let term = match user_field(query.fields.as_slice(), field_name) {
                    Some(value) => field_value_to_term(field_name, value, scope, analysis).await?,
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
                if resolved
                    .descriptor
                    .with()
                    .iter()
                    .all(|(n, _)| n != field.name)
                {
                    return Err(AnalyzeError::UnknownField {
                        concept: concept_name.clone(),
                        field: field.name.clone(),
                    });
                }
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
                },
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
                return Err(AnalyzeError::ClaimWithoutFields {
                    domain: domain.clone(),
                });
            }
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term_for_query(&this));
            for field in &body_fields {
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                },
                this,
                name: None,
            })
        }
        HeadName::Uri(uri) => Err(AnalyzeError::UnsupportedFieldValue {
            field: uri.clone(),
            form: "URI head in query (not yet implemented in Stage 2.1)",
        }),
    }
}

fn user_field<'a>(fields: &'a [Field], name: &str) -> Option<&'a FieldValue> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

/// Reserved body field names that don't correspond to schema
/// fields: `this:` (entity selection), `..:` (rest-of-attributes
/// retraction marker).
fn is_meta_field(name: &str) -> bool {
    matches!(name, "this" | "..")
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

// ---------------------------------------------------------------- //
// Phase 3 — build mutation Applications                            //
// ---------------------------------------------------------------- //

async fn build_assertion_application<R: Resolver>(
    assertion: &Assertion,
    scope: &Scope<'_, R>,
    analysis: &mut Analysis,
) -> Result<Application, AnalyzeError> {
    let head_label = match &assertion.head.name {
        HeadName::Concept(name) => name.clone(),
        HeadName::Claim(domain) => domain.clone(),
        HeadName::Uri(uri) => uri.clone(),
    };

    if assertion.fields.is_empty() {
        return Err(AnalyzeError::AssertionWithoutFields { head: head_label });
    }

    let (this, name) = derive_head_intent(&assertion.fields, assertion.anchor.as_ref())?;
    let this_term = this_term_for_assertion(&this, &name, &assertion.fields, analysis)?;

    match &assertion.head.name {
        HeadName::Concept(concept_name) => {
            let resolved = scope
                .resolve_concept(concept_name)
                .await
                .map_err(|e| AnalyzeError::ResolverFailed {
                    context: format!("concept {concept_name:?}"),
                    reason: e.message,
                })?
                .ok_or_else(|| AnalyzeError::UnknownConcept {
                    name: concept_name.clone(),
                })?;
            let mut terms = Parameters::new();
            terms.insert("this".into(), this_term);
            let mut user_fields: BTreeMap<&str, &FieldValue> = BTreeMap::new();
            for field in &assertion.fields {
                if is_meta_field(&field.name) {
                    continue;
                }
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
                    concept: concept_name.clone(),
                    field: unknown.to_owned(),
                });
            }
            Ok(Application::Concept {
                query: ConceptQuery {
                    terms,
                    predicate: resolved.descriptor,
                },
                this,
                name,
            })
        }
        HeadName::Claim(domain) => {
            let mut parameters = Parameters::new();
            parameters.insert("this".into(), this_term);
            for field in &assertion.fields {
                if is_meta_field(&field.name) {
                    continue;
                }
                validate_claim_attribute(domain, &field.name)?;
                let term = field_value_to_term(&field.name, &field.value, scope, analysis).await?;
                parameters.insert(field.name.clone(), term);
            }
            Ok(Application::Domain {
                application: DomainApplication {
                    domain: domain.clone(),
                    parameters,
                },
                this,
                name,
            })
        }
        HeadName::Uri(uri) => Err(AnalyzeError::UnsupportedFieldValue {
            field: uri.clone(),
            form: "URI head in assertion (not yet implemented in Stage 2.1)",
        }),
    }
}

/// Derive the head's source-form intent — `(ThisIntent, name)`
/// — from an expression's body and optional value-side anchor.
///
/// Under the new grammar the head carries no binding token; the
/// two intent axes live in the body and value side:
///
/// - **Entity selection** — the body's `this:` field. Mapping:
///   - omitted → [`ThisIntent::Derived`]
///   - `?var` → [`ThisIntent::Variable(var)`]
///   - `did:key:…` / `id:…` / `db:…` → [`ThisIntent::Uri(entity)`]
///   - bare symbol → name lookup; Stage 2.4 will resolve through
///     the name table. Today: error.
///
/// - **Naming** — the `&name` on the value side, captured by the
///   parser as `Anchor`. Returned as `Some(name)` when present.
///
/// The two are independent: every combination is meaningful
/// (e.g. `person!: &alice\n  this: did:key:zX` → publish `id:alice`
/// pointing at zX without producing a new entity).
fn derive_head_intent(
    fields: &[Field],
    anchor: Option<&Anchor>,
) -> Result<(ThisIntent, Option<String>), AnalyzeError> {
    let name = anchor.map(|a| a.name.clone());
    let this = match fields.iter().find(|f| f.name == "this") {
        None => ThisIntent::Derived,
        Some(field) => match &field.value {
            FieldValue::Variable(v) => ThisIntent::Variable(v.clone()),
            FieldValue::Uri(uri) => {
                let entity: Entity =
                    uri.parse()
                        .map_err(|e: dialog_artifacts::DialogArtifactsError| {
                            AnalyzeError::InvalidSubjectUri {
                                subject: uri.clone(),
                                reason: e.to_string(),
                            }
                        })?;
                ThisIntent::Uri(entity)
            }
            FieldValue::Symbol(_)
            | FieldValue::Literal(_)
            | FieldValue::Blank
            | FieldValue::Nested(_) => {
                return Err(AnalyzeError::UnsupportedFieldValue {
                    field: "this".into(),
                    form: "expected `?var` or a URI (`did:key:…`, `id:…`, `db:…`); \
                           bare-symbol name lookup arrives in Stage 2.4",
                });
            }
        },
    };
    Ok((this, name))
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
fn attribute_application(
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
fn concept_application(
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
/// field of the concept being defined, plus the
/// `dialog.meta/concept` marker (so branch-wide `concept:` queries
/// can find every concept entity) and optional name and description
/// fields.
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

/// What to put in the `this` slot of a mutation [`Application`].
///
/// Driven by the entity-selection axis (`this`) and the optional
/// published name (`name`). The two axes are orthogonal — both
/// can be present, both can be absent.
///
/// - `Derived` + no `name`: mint a body-content-derived entity.
/// - `Derived` + `name`: body-derived entity. The
///   `dialog.meta/name` claim on `id:<name>` is emitted by the
///   planner from `ApplicationPlan::name`, not as a parameter
///   on the predicate. Also registers the name → entity binding
///   in `analysis.declarations` so duplicate-name checks across
///   heads catch overlaps.
/// - `Variable(name)` already in `analysis.variables`: substitute
///   the registered entity.
/// - `Variable(name)` not yet known: if there's no query
///   binding for it, mint a body-derived entity and register it
///   in `analysis.variables` so subsequent uses share the
///   entity. If a query binding exists, leave as
///   `Term::Variable` — planning will substitute from the
///   query frame.
/// - `Uri(entity)`: substitute directly. With `name`, this is
///   the "publish a name pointing at an existing entity" form.
fn this_term_for_assertion(
    this: &ThisIntent,
    name: &Option<String>,
    fields: &[Field],
    analysis: &mut Analysis,
) -> Result<Term<dialog_query::Any>, AnalyzeError> {
    Ok(match this {
        ThisIntent::Derived => {
            let entity = Entity::of(&body_digest(fields));
            if let Some(name) = name {
                if let Some(prior) = analysis.declarations.get(name)
                    && prior != &entity
                {
                    return Err(AnalyzeError::DuplicateName { name: name.clone() });
                }
                analysis.declarations.insert(name.clone(), entity.clone());
            }
            Term::Constant(Value::Entity(entity))
        }
        ThisIntent::Variable(var) => {
            if let Some(entity) = analysis.variables.get(var) {
                Term::Constant(Value::Entity(entity.clone()))
            } else if query_binds(analysis, var) {
                // Bound at planning time from query results.
                Term::<dialog_query::Any>::var(var)
            } else {
                // First introduction — mint a body-derived
                // entity and register it for later expressions
                // that share `?name`.
                let entity = Entity::of(&body_digest(fields));
                analysis.variables.insert(var.clone(), entity.clone());
                Term::Constant(Value::Entity(entity))
            }
        }
        ThisIntent::Uri(entity) => Term::Constant(Value::Entity(entity.clone())),
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
        FieldValue::Symbol(name) => {
            // Bare lowercase symbol — name-table lookup. Same
            // resolution order as the old `.bookmark` form:
            //   1. Doc-local declarations (head anchor from the
            //      same document — `concept!: &foo` or
            //      `attribute!: &foo`).
            //   2. Doc-local attribute by name.
            //   3. Branch entity with `dialog.meta/name = name`.
            if let Some(entity) = scope.lookup_entity(name) {
                Term::Constant(Value::Entity(entity))
            } else if let Some(resolved) =
                scope
                    .resolve_attribute(name)
                    .await
                    .map_err(|e| AnalyzeError::ResolverFailed {
                        context: format!("symbol {name}"),
                        reason: e.message,
                    })?
            {
                Term::Constant(Value::Entity(resolved.entity))
            } else if let Some(entity) = scope.resolve_named_entity(name).await.map_err(|e| {
                AnalyzeError::ResolverFailed {
                    context: format!("symbol {name}"),
                    reason: e.message,
                }
            })? {
                Term::Constant(Value::Entity(entity))
            } else {
                return Err(AnalyzeError::UnknownBookmark {
                    field: field_name.into(),
                    bookmark: name.clone(),
                });
            }
        }
        FieldValue::Uri(uri) => {
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
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

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
        async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolverError> {
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
    async fn it_rejects_empty_document() {
        let syntax = Syntax {
            expressions: Vec::new(),
            range: lsp_types::Range::default(),
        };
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::EmptyDocument));
    }

    /// `attribute!: &foo` declares a content-derived attribute
    /// entity in `declarations`, an Assert statement, no query,
    /// no requires.
    #[dialog_common::test]
    async fn it_declares_a_single_attribute_assertion() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
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
    /// referencing them via bare symbols. Concept body resolution
    /// must hit the in-doc index, not the (Noop) outer resolver.
    #[dialog_common::test]
    async fn it_resolves_attributes_referenced_by_a_concept_in_the_same_doc() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
attribute!: &person-age
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one
  description: "Person's age"
concept!: &person
  description: "Person concept"
  with:
    name: person-name
    age:  person-age
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        assert!(analysis.declarations.contains_key("person-name"));
        assert!(analysis.declarations.contains_key("person-age"));
        assert!(analysis.declarations.contains_key("person"));
        assert!(analysis.query.is_none());
        // 3 statements — 2 attribute + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
    }

    /// `concept!` `with:` accepts inline attribute definitions
    /// alongside bare-symbol references. Each inline definition
    /// becomes its own `Statement::Assert` so the attribute
    /// surfaces in `attribute:` queries; the inline attrs are
    /// anonymous (no `dialog.meta/name` claim, since the field
    /// key is the concept's local name, not a global label).
    #[dialog_common::test]
    async fn it_emits_inline_attribute_definitions_from_concept_with() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "A person"
  with:
    name:
      description: "Name of the person"
      the:         xyz.tonk.person/name
      as:          Text
      cardinality: one
    age:
      description: "Age of the person"
      the:         xyz.tonk.person/age
      as:          UnsignedInteger
      cardinality: one
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // 3 statements: 2 inline attrs + 1 concept.
        assert_eq!(analysis.mutate.statements.len(), 3);
        // First two are anonymous attributes (no published name).
        for i in 0..2 {
            let Statement::Assert(Application::Concept { name, .. }) =
                &analysis.mutate.statements[i]
            else {
                panic!("expected Assert(Concept) for inline attr {i}");
            };
            assert!(name.is_none(), "inline attr should have no published name");
        }
        // Third is the concept itself, anchored as `&person`.
        let Statement::Assert(Application::Concept { name, .. }) = &analysis.mutate.statements[2]
        else {
            panic!("expected Assert(Concept) for concept");
        };
        assert_eq!(name.as_deref(), Some("person"));
    }

    /// A bare symbol in field-value position resolves through the
    /// in-doc concept table — referencing `person` should find a
    /// `concept!: &person` defined earlier in the same document,
    /// not error as if `person` were an unknown attribute.
    #[dialog_common::test]
    async fn it_resolves_bare_symbol_to_in_doc_concept() {
        let syntax = must_parse(
            r#"
concept!: &person
  description: "A person"
  with:
    name:
      description: "Name of the person"
      the:         xyz.tonk.person/name
      as:          Text
      cardinality: one
concept!: &view
  description: "A view"
  with:
    source:
      description: "Source concept"
      the:         dialog.view/source
      as:          Entity
      cardinality: one
view!: &title
  source: person
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // The view's `source` term should be a constant entity
        // pointing at the `person` concept's entity, not a
        // variable.
        let person_entity = analysis
            .declarations
            .get("person")
            .expect("person was declared");
        let view_assert = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = view_assert else {
            panic!("expected view assertion");
        };
        match query.terms.get("source") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert_eq!(e, person_entity, "source should resolve to person's entity");
            }
            other => panic!("source should be a constant entity, got {other:?}"),
        }
    }

    /// Attribute definitions without `description` are rejected.
    /// Both `attribute!: &foo` form and inline-in-`concept!` form.
    #[dialog_common::test]
    async fn it_rejects_attribute_definition_without_description() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/foo
  as:          Text
  cardinality: one
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(&err, AnalyzeError::InvalidAttributeBody { reason } if reason.contains("description")),
            "expected InvalidAttributeBody about description, got {err:?}"
        );
    }

    /// Variable-form `attribute!:` with `this: ?foo` (no anchor)
    /// lands in `variables`, not `declarations`, and does NOT
    /// emit a `dialog.meta/name` claim (the name is doc-scoped
    /// only).
    #[dialog_common::test]
    async fn it_keeps_variable_form_attribute_doc_scoped() {
        let syntax = must_parse(
            r#"
attribute!:
  this:        ?person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // Variable-form `this:` (no anchor) registers the name
        // in `variables`, not `declarations`. The meta-pass
        // resolves the variable to the body-derived attribute
        // entity and stores it for cross-expression sharing.
        assert!(analysis.declarations.is_empty());
        assert!(analysis.variables.contains_key("person-name"));
        let Statement::Assert(Application::Concept { query, name, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        // Variable-form attributes publish no name (no anchor),
        // so neither `name` nor any predicate `name` term is set.
        assert!(query.terms.get("name").is_none());
        assert!(name.is_none());
    }

    /// `attribute!: &foo` (anchored): the head's `name` slot
    /// records the published name. The planner emits the
    /// `dialog.meta/name` claim on `id:<name>`, not as a
    /// parameter on the predicate.
    #[dialog_common::test]
    async fn it_records_published_name_for_anchored_attribute() {
        let syntax = must_parse(
            r#"
attribute!: &person-name
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "Person's name"
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let Statement::Assert(Application::Concept { query, name, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert!(query.terms.get("name").is_none());
        assert_eq!(name.as_deref(), Some("person-name"));
    }

    /// Two meta heads of different kinds (`attribute!` and
    /// `concept!`) that share a bookmark name → `DuplicateName`.
    /// Phase 1 sees both heads declare `a` and the second
    /// `declare` returns `Some(prior_entity)`, triggering the
    /// error. Only meta heads register declarations in Phase 1
    /// — non-meta heads defer their entity to Phase 3 and so
    /// are not checked for name collisions today.
    #[dialog_common::test]
    async fn it_rejects_duplicate_meta_bookmark_anchors() {
        // The concept's `with: { x: a }` references the `a`
        // attribute defined just above, so concept-body
        // resolution succeeds and Phase 1 reaches the second
        // `declare("a", …)` call which finds the prior entry.
        let syntax = must_parse(
            r#"
attribute!: &a
  the:         x.y/a
  as:          Text
  cardinality: one
  description: "A"
concept!: &a
  with:
    x: a
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(
            matches!(err, AnalyzeError::DuplicateName { .. }),
            "expected DuplicateName, got {err:?}"
        );
    }

    /// A bookmark anchor and a `this: ?var` that share a name →
    /// `NameShadowing`.
    #[dialog_common::test]
    async fn it_rejects_anchor_and_this_variable_with_same_name() {
        let syntax = must_parse(
            r#"
attribute!: &foo
  the:         x.y/a
  as:          Text
  cardinality: one
  description: "A"
attribute!:
  this:        ?foo
  the:         x.y/b
  as:          Text
  cardinality: one
  description: "B"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::NameShadowing { .. }));
    }

    /// Pure-query document: `Analysis::query` is `Some`, no
    /// statements, no requires.
    #[dialog_common::test]
    async fn it_analyzes_a_pure_query_document() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
"#,
        );
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
    async fn it_joins_query_and_assertion_via_this_variable() {
        let syntax = must_parse(
            r#"
person:
  this: ?alice
  name: "Alice"
person!:
  this: ?alice
  name: "Renamed"
"#,
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
    async fn it_rejects_unbound_mutation_variable() {
        let syntax = must_parse(
            r#"
person!:
  this: ?ghost
  name: ?nope
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let err = analyze(&syntax, &resolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnboundMutationVariable { .. }));
    }

    /// Concept retraction via `..: _`: blank terms for every
    /// field, the `this` term carries the URI-resolved entity.
    #[dialog_common::test]
    async fn it_blanks_every_field_for_concept_retraction() {
        let syntax = must_parse(
            r#"
person!:
  this: did:key:z6MkfpAVgERtxfLXxr8wpJp3CQpXi2VZkAjJBgvw9q5tGBkv
  ..: _
"#,
        );
        let resolver = fixed_concept(
            "person",
            &[
                ("name", "io.gozala.person/name"),
                ("age", "io.gozala.person/age"),
            ],
        );
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        // Stage 2.1: `..: _` is parsed but not yet wired to
        // produce `Statement::Retract`. The expression goes
        // through as `Statement::Assert` whose every field is
        // blank because the user supplied no values. Stage 2.8
        // turns this into a real retraction.
        let Statement::Assert(Application::Concept { query: q, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept) — Stage 2.1 placeholder");
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
    async fn it_errors_on_unknown_concept() {
        let syntax = must_parse(
            r#"
nope:
  field: "x"
"#,
        );
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::UnknownConcept { .. }));
    }

    /// Built-in `attribute:` resolves without a branch resolver
    /// — the registry is consulted before the inner resolver, so
    /// the LSP (which uses [`NoopResolver`]) gets autocomplete /
    /// diagnostics for built-ins for free.
    #[dialog_common::test]
    async fn it_resolves_builtin_attribute_under_noop_resolver() {
        let syntax = must_parse(
            r#"
attribute:
  this: ?a
  description: ?d
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        assert_eq!(q.queries.len(), 1);
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        // The four anonymous-attribute fields — id/type/
        // cardinality/description — must all be present in the
        // unified term map. `name` is intentionally not in the
        // built-in `attribute:` view (only bookmark-form attrs
        // carry a `dialog.meta/name` claim).
        for field in ["id", "type", "cardinality", "description"] {
            assert!(query.terms.contains(field), "missing {field}");
        }
    }

    /// Built-in `branch:` (and other Rust-defined repository
    /// concepts) resolve through the registry, not the resolver,
    /// even though they have no branch-side `concept!`
    /// definition.
    #[dialog_common::test]
    async fn it_resolves_builtin_branch_under_noop_resolver() {
        let syntax = must_parse(
            r#"
branch:
  this: ?b
  name: ?name
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        assert!(query.terms.contains("name"));
        assert!(query.terms.contains("origin"));
    }

    /// Built-in `attribute:` empty-body query surfaces every
    /// well-known field as a defaulted variable. Documents the
    /// "fields you didn't mention default to ?field" behavior
    /// applied to a built-in.
    #[dialog_common::test]
    async fn it_defaults_every_attribute_field_to_a_variable_on_empty_body() {
        let syntax = must_parse("attribute:\n");
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        for field in ["id", "type", "cardinality", "description"] {
            assert!(query.terms.contains(field), "missing {field}");
        }
    }

    /// Claim head with no fields → `ClaimWithoutFields`.
    #[dialog_common::test]
    async fn it_errors_on_claim_without_fields() {
        let syntax = must_parse("xyz.tonk:\n");
        let err = analyze(&syntax, &NoopResolver).await.unwrap_err();
        assert!(matches!(err, AnalyzeError::ClaimWithoutFields { .. }));
    }

    /// Claim heads build a synthesized predicate with one
    /// `<domain>/<field>` attribute per parameter.
    #[dialog_common::test]
    async fn it_synthesizes_descriptor_for_claim_head() {
        let syntax = must_parse(
            r#"
xyz.tonk:
  role: ?role
  contact: "alice"
"#,
        );
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

    /// Anonymous-head queries (`person:`) need `this` as a
    /// named variable, not a blank — otherwise dialog's engine
    /// fails with `UnboundVariable { variable_name: "this" }`
    /// at evaluation. Regression for that crash.
    #[dialog_common::test]
    async fn it_binds_this_as_variable_for_anonymous_query_head() {
        let syntax = must_parse(
            r#"
person:
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        assert!(matches!(
            query.terms.get("this"),
            Some(Term::Variable { name: Some(_), .. })
        ));
    }

    /// `name` is a built-in concept resolvable without a branch
    /// — same as `attribute` and `concept`. Backed by the single
    /// `dialog.meta/name` attribute (cardinality one).
    #[dialog_common::test]
    async fn it_resolves_builtin_name_concept() {
        let syntax = must_parse(
            r#"
name:
  this: ?n
  entity: ?e
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        let q = analysis.query.as_ref().unwrap();
        let Application::Concept { query, .. } = &q.queries[0] else {
            panic!("expected Concept application");
        };
        // `entity` field comes from the `Name` concept's `with:`.
        assert!(query.terms.contains("entity"));
    }

    /// The concept-of-concept built-in resolves to `db:concept`
    /// — *not* the legacy `concept:concept`.
    #[dialog_common::test]
    async fn it_uses_db_concept_uri_for_concept_marker() {
        // Asserting any concept emits a marker claim
        // `(this, dialog.meta/concept, db:concept)`. We can read
        // it back by inspecting the `concept` term of the
        // emitted concept-head application.
        let syntax = must_parse(
            r#"
concept!: &foo
  description: "x"
  with:
    bar:
      description: "y"
      the: x.y/bar
      as: Text
      cardinality: one
"#,
        );
        let analysis = analyze(&syntax, &NoopResolver).await.unwrap();
        // Last statement is the concept itself — its `concept`
        // term should be the marker entity.
        let last = analysis.mutate.statements.last().unwrap();
        let Statement::Assert(Application::Concept { query, .. }) = last else {
            panic!("expected Concept application for concept");
        };
        match query.terms.get("concept") {
            Some(Term::Constant(Value::Entity(e))) => {
                assert_eq!(
                    e.to_string(),
                    "db:concept",
                    "concept marker should be db:concept"
                );
            }
            other => panic!("expected concept term to be db:concept entity, got {other:?}"),
        }
    }

    /// The anchor publication (`person!: &alice`) is recorded
    /// as `name: Some("alice")` on the `Application`. The `this`
    /// intent stays `Derived` since the body produces the entity.
    #[dialog_common::test]
    async fn it_records_anchor_publication_on_application() {
        let syntax = must_parse(
            r#"
person!: &alice
  name: "Alice"
"#,
        );
        let resolver = fixed_concept("person", &[("name", "io.gozala.person/name")]);
        let analysis = analyze(&syntax, &resolver).await.unwrap();
        assert_eq!(analysis.mutate.statements.len(), 1);
        let Statement::Assert(Application::Concept { name, this, .. }) =
            &analysis.mutate.statements[0]
        else {
            panic!("expected Assert(Concept)");
        };
        assert_eq!(name.as_deref(), Some("alice"));
        assert!(matches!(this, ThisIntent::Derived));
    }
}
