//! [`Resolver`] — branch-side name lookup vocabulary for the
//! analyzer. The analyzer's `&str`-based world (head names, bare
//! symbols) lives one layer above the typed-reference world the
//! [`tonk_schema::resolution`] chain builders speak; `Resolver`
//! adapts between them so analyzer call sites don't construct
//! `ConceptReference` / `AttributeReference` inline.
//!
//! Every live implementation is the blanket impl over
//! [`Environment`] below. The host (evaluator, language server)
//! constructs an `Environment` once per request — via
//! [`tonk_schema::resolution::source_env`] for live `(Source,
//! QueryEnv)` pairs or by implementing `Environment` directly —
//! and hands it to [`super::analyze`]. The blanket impl translates
//! `&str` → typed reference and dispatches to the chain.
//!
//! [`NoopResolver`] is the document-only path: useful for tests
//! and for analyzer surfaces where no branch is available.

use async_trait::async_trait;
use dialog_artifacts::Entity;
use dialog_common::ConditionalSync;

use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, Environment,
    NamedReference, ResolveError,
};

/// Look up names against a backing store (typically the branch).
///
/// The analyzer calls this when it encounters a concept name in
/// head position or a bare-symbol reference in field-value
/// position. Anything that implements [`Environment`] auto-implements
/// `Resolver` via the blanket impl below; [`NoopResolver`] is the
/// document-only one.
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

/// Blanket impl: every [`Environment`] is a [`Resolver`]. The
/// analyzer's `&str` vocabulary maps to the typed `*Reference`
/// shape the chain builders consume.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<E: Environment + ConditionalSync + ?Sized> Resolver for E {
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        Environment::resolve_concept(
            self,
            ConceptReference::from(NamedReference(name.to_owned())),
        )
        .await
    }

    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Environment::resolve_attribute(
            self,
            AttributeReference::from(NamedReference(name.to_owned())),
        )
        .await
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        Environment::resolve_attribute(self, AttributeReference::from(entity.clone())).await
    }

    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        Environment::resolve_named_entity(self, name).await
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
