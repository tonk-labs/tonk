//! [`Scope`] — layered name index built during analysis.
//!
//! Holds the live source the analyzer resolves against. env is
//! *not* stored, it is passed to each resolve_* call so the
//! resolution chain takes it at `.perform` time (the dialog
//! idiom). Source is a borrowed handle to a branch / txn that
//! lives for the analyze call; env is a per-execution context.

use std::collections::HashMap;

use dialog_artifacts::Entity;
use parking_lot::Mutex;

use super::error::{AnalyzeError, AnalyzeErrorKind};
use tonk_schema::concept::{QueryEnv, lookup_named_entity};
use tonk_schema::query_source::Source;
use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, NamedReference,
    ResolveError,
};

/// Layered name index built during analysis.
///
/// Each map is wrapped in a `parking_lot::Mutex` so the analyzer
/// can mutate the scope from inside `&self` methods (the same
/// scope is shared across the analyzer's three phases). The
/// guards are `Send`, so axum handlers stay happy on native; on
/// wasm the runtime is single-threaded and the lock is
/// uncontended. Critical sections never cross an `.await`:
/// `lookup_entity` and `resolve_*` drop their guards before
/// recursing into the branch resolution chain.
pub(crate) struct Scope<'a> {
    source: Source<'a>,
    /// Anchor/variable → entity for non-meta head bindings
    /// (every head except `attribute!` / `concept!` whose
    /// declarations live in the dedicated maps below). One map
    /// per source (anchor vs variable), surfaced separately
    /// because `Analysis` keeps them separate.
    pub(crate) declarations: Mutex<HashMap<String, Entity>>,
    pub(crate) variables: Mutex<HashMap<String, Entity>>,
    /// `attribute!` definitions made in the document, indexed by
    /// the anchor/variable name on the head. Used by later
    /// `concept!` heads in the same document so their `with:`
    /// map can resolve bare-symbol / `?var` references against
    /// uncommitted attributes.
    pub(crate) in_doc_attributes: Mutex<HashMap<String, AttributeDefinition>>,
    /// `concept!` definitions made in the document, indexed by
    /// the anchor/variable name on the head. Used by later
    /// `person!: &alice` heads in the same document.
    pub(crate) in_doc_concepts: Mutex<HashMap<String, ConceptDefinition>>,
    /// Reverse index: attribute entity → resolved attribute.
    /// Used when a concept body references an attribute via URI
    /// instead of by name.
    pub(crate) in_doc_attributes_by_entity: Mutex<HashMap<String, AttributeDefinition>>,
    /// Reverse index: concept entity → resolved concept.
    pub(crate) in_doc_concepts_by_entity: Mutex<HashMap<String, ConceptDefinition>>,
}

impl<'a> Scope<'a> {
    /// Borrow the resolution [`Source`] the scope was constructed
    /// over — `rule!: ..: _` retracts need it to read the stored
    /// `dialog.effect/source` bytes off the branch.
    pub(crate) fn source(&self) -> &Source<'a> {
        &self.source
    }

    pub(crate) fn new(source: Source<'a>) -> Self {
        Self {
            source,
            declarations: Mutex::new(HashMap::new()),
            variables: Mutex::new(HashMap::new()),
            in_doc_attributes: Mutex::new(HashMap::new()),
            in_doc_concepts: Mutex::new(HashMap::new()),
            in_doc_attributes_by_entity: Mutex::new(HashMap::new()),
            in_doc_concepts_by_entity: Mutex::new(HashMap::new()),
        }
    }

    /// Record an anchor-form head's entity.
    pub(crate) fn declare(
        &self,
        name: &str,
        entity: Entity,
        range: lsp_types::Range,
    ) -> Result<(), AnalyzeError> {
        if self.variables.lock().contains_key(name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::NameShadowing {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        let prior = self.declarations.lock().insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::DuplicateName {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        Ok(())
    }

    /// Record a variable-form head's entity.
    pub(crate) fn bind_variable(
        &self,
        name: &str,
        entity: Entity,
        range: lsp_types::Range,
    ) -> Result<(), AnalyzeError> {
        if self.declarations.lock().contains_key(name) {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::NameShadowing {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        let prior = self.variables.lock().insert(name.to_owned(), entity);
        if prior.is_some() {
            return Err(AnalyzeError::at(
                AnalyzeErrorKind::DuplicateName {
                    name: name.to_owned(),
                },
                range,
            ));
        }
        Ok(())
    }

    /// Record an in-document `attribute!` definition for the
    /// given declaration / variable name.
    pub(crate) fn record_attribute(&self, name: Option<&str>, attribute: AttributeDefinition) {
        if let Some(name) = name {
            self.in_doc_attributes
                .lock()
                .insert(name.to_owned(), attribute.clone());
        }
        self.in_doc_attributes_by_entity
            .lock()
            .insert(attribute.entity.to_string(), attribute);
    }

    /// Record an in-document `concept!` definition.
    pub(crate) fn record_concept(&self, name: Option<&str>, concept: ConceptDefinition) {
        if let Some(name) = name {
            self.in_doc_concepts
                .lock()
                .insert(name.to_owned(), concept.clone());
        }
        self.in_doc_concepts_by_entity
            .lock()
            .insert(concept.entity.to_string(), concept);
    }

    /// Look up the entity bound to an anchor or `?var` name,
    /// regardless of which side it lives on. Returns `None` if
    /// the name isn't known yet.
    pub(crate) fn lookup_entity(&self, name: &str) -> Option<Entity> {
        if let Some(e) = self.declarations.lock().get(name) {
            return Some(e.clone());
        }
        self.variables.lock().get(name).cloned()
    }

    pub(crate) async fn resolve_concept<Env: QueryEnv>(
        &self,
        name: &str,
        env: &Env,
    ) -> Result<Option<ConceptDefinition>, ResolveError> {
        // Drop the lock before awaiting the fallback resolver:
        // holding a guard across an await could deadlock if the
        // resolver came back to us.
        if let Some(found) = self.in_doc_concepts.lock().get(name).cloned() {
            return Ok(Some(found));
        }
        if let Some(found) = tonk_schema::builtin::lookup_concept(name) {
            return Ok(Some(found));
        }
        ConceptReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(env)
            .await
    }

    /// Resolve a bare symbol to *any* in-doc or branch entity
    /// with that name. Used by [`super::field::field_value_to_term`]
    /// when the symbol doesn't match an attribute (concepts and
    /// previously-asserted instances also have `dialog.meta/name`
    /// claims and should be reachable).
    pub(crate) async fn resolve_named_entity<Env: QueryEnv>(
        &self,
        name: &str,
        env: &Env,
    ) -> Result<Option<Entity>, ResolveError> {
        if let Some(found) = self.in_doc_concepts.lock().get(name).cloned() {
            return Ok(Some(found.entity));
        }
        lookup_named_entity(name, self.source.clone(), env).await
    }

    pub(crate) async fn resolve_attribute<Env: QueryEnv>(
        &self,
        name: &str,
        env: &Env,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        if let Some(found) = self.in_doc_attributes.lock().get(name).cloned() {
            return Ok(Some(found));
        }
        AttributeReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(env)
            .await
    }

    pub(crate) async fn resolve_attribute_by_entity<Env: QueryEnv>(
        &self,
        entity: &Entity,
        env: &Env,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        let key = entity.to_string();
        if let Some(found) = self.in_doc_attributes_by_entity.lock().get(&key).cloned() {
            return Ok(Some(found));
        }
        AttributeReference::from(entity.clone())
            .resolve(self.source.clone())
            .perform(env)
            .await
    }
}
