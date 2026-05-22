//! [`Resolver`] trait — branch-side name lookup. The analyzer
//! calls this when it encounters a concept name in head position
//! or a bare-symbol reference in field-value position.
//!
//! [`NoopResolver`] is provided for document-only analysis paths
//! (no branch) and for unit tests.
//!
//! The resolved values are [`ConceptDefinition`] and
//! [`AttributeDefinition`] from [`crate::resolution`] — the
//! single resolved-concept / resolved-attribute types in
//! `tonk-schema`. `Resolver` is the seam the language server
//! drives (`analyze(syntax, &NoopResolver)`); it stays as the
//! analyzer's vocabulary, with a blanket impl over
//! [`tonk_introspect::BranchIntrospection`] so a branch-backed
//! host satisfies it for free.

use async_trait::async_trait;
use dialog_artifacts::Entity;

use crate::mutation::ConceptDescriptor;
use crate::resolution::{AttributeDefinition, ConceptDefinition, ResolveError};

/// Look up names against a backing store (typically the branch).
///
/// The analyzer calls this when it encounters a concept name in
/// head position or a bare-symbol reference in field-value
/// position. Every [`tonk_introspect::BranchIntrospection`] is a
/// `Resolver` via the blanket impl below, so a branch-backed host
/// satisfies it without a separate adapter.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Resolver {
    /// Resolve a concept by name (or `Ok(None)` if not found).
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError>;

    /// Resolve an attribute by name. Used for field-value
    /// references (`field: person-name`) and by `concept!`'s
    /// `with:` map.
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError>;

    /// Resolve an attribute by its entity URI.
    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError>;

    /// Resolve any entity by its `dialog.meta/name` claim.
    /// Powers bare-symbol references that point at concepts or
    /// concept instances rather than attributes — `view!`
    /// referencing `person` should pick up a `concept!: &person`
    /// definition or a `person!: &foo` instance with `name: foo`.
    /// Returns the entity URI; the analyzer doesn't need a
    /// descriptor here because the symbol only uses the entity
    /// as a constant in field-value position.
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError>;
}

/// Convert from the introspection trait's resolved-concept
/// type. `tonk_introspect::ResolvedConcept` carries a plain
/// dialog descriptor plus a `transient` flag; [`ConceptDefinition`]
/// folds the flag into the durability-tagged descriptor.
fn definition_from_introspection(c: tonk_introspect::ResolvedConcept) -> ConceptDefinition {
    let descriptor = if c.transient {
        ConceptDescriptor::Transient(c.descriptor)
    } else {
        ConceptDescriptor::Durable(c.descriptor)
    };
    ConceptDefinition {
        entity: c.entity,
        descriptor,
    }
}

/// Convert from the introspection trait's resolved-attribute
/// type. Both carry the same `(entity, descriptor)` pair.
fn attribute_from_introspection(a: tonk_introspect::ResolvedAttribute) -> AttributeDefinition {
    AttributeDefinition {
        entity: a.entity,
        descriptor: a.descriptor,
    }
}

/// Blanket impl: every [`tonk_introspect::BranchIntrospection`]
/// is also a [`Resolver`]. The four `resolve_*` methods forward
/// to their `lookup_*` counterparts on the introspection trait,
/// converting the result types at the boundary.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl<T> Resolver for T
where
    T: tonk_introspect::BranchIntrospection + ?Sized + Sync,
{
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        Ok(self
            .lookup_concept(name)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(definition_from_introspection))
    }
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(self
            .lookup_attribute(name)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(attribute_from_introspection))
    }
    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(self
            .lookup_attribute_by_entity(entity)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(attribute_from_introspection))
    }
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        self.lookup_named_entity(name)
            .await
            .map_err(|e| ResolveError::query(e.message))
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl<T> Resolver for T
where
    T: tonk_introspect::BranchIntrospection + ?Sized,
{
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        Ok(self
            .lookup_concept(name)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(definition_from_introspection))
    }
    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(self
            .lookup_attribute(name)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(attribute_from_introspection))
    }
    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(self
            .lookup_attribute_by_entity(entity)
            .await
            .map_err(|e| ResolveError::query(e.message))?
            .map(attribute_from_introspection))
    }
    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        self.lookup_named_entity(name)
            .await
            .map_err(|e| ResolveError::query(e.message))
    }
}

/// A [`Resolver`] that always returns `None`. Convenient for
/// document-only analysis paths and unit tests.
pub struct NoopResolver;

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Resolver for NoopResolver {
    async fn resolve_concept(
        &self,
        _name: &str,
    ) -> Result<Option<ConceptDefinition>, ResolveError> {
        Ok(None)
    }
    async fn resolve_attribute(
        &self,
        _name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(None)
    }
    async fn resolve_attribute_by_entity(
        &self,
        _entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Ok(None)
    }
    async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolveError> {
        Ok(None)
    }
}
