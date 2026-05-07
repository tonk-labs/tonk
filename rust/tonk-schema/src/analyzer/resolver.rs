//! [`Resolver`] trait — branch-side name lookup. The analyzer
//! calls this when it encounters a concept name in head position
//! or a bare-symbol reference in field-value position.
//!
//! [`NoopResolver`] is provided for document-only analysis paths
//! (no branch) and for unit tests.

use async_trait::async_trait;
use dialog_artifacts::Entity;
use dialog_query::{AttributeDescriptor, ConceptDescriptor};
use thiserror::Error;

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
/// head position or a bare-symbol reference in field-value
/// position.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Resolver {
    /// Resolve a concept by name (or `Ok(None)` if not found).
    async fn resolve_concept(&self, name: &str) -> Result<Option<ResolvedConcept>, ResolverError>;

    /// Resolve an attribute by name. Used for field-value
    /// references (`field: person-name`) and by `concept!`'s
    /// `with:` map.
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
    /// Powers bare-symbol references that point at concepts or
    /// concept instances rather than attributes — `view!`
    /// referencing `person` should pick up a `concept!: &person`
    /// definition or a `person!: &foo` instance with `name: foo`.
    /// Returns the entity URI; the analyzer doesn't need a
    /// descriptor here because the symbol only uses the entity
    /// as a constant in field-value position.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolverError>;
}

/// Opaque error from a [`Resolver`] implementation. The analyzer
/// wraps this into [`crate::analyzer::AnalyzeError::ResolverFailed`].
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
