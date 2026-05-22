//! [`Resolver`] trait — branch-side name lookup. The analyzer
//! calls this when it encounters a concept name in head position
//! or a bare-symbol reference in field-value position.
//!
//! [`NoopResolver`] is provided for document-only analysis paths
//! (no branch) and for unit tests. [`SourceResolver`] is the
//! live-environment implementation: it wraps a
//! [`crate::query_source::Source`] and a query environment and
//! delegates every lookup to the [`crate::resolution`] surface.
//!
//! The resolved values are [`ConceptDefinition`] and
//! [`AttributeDefinition`] from [`crate::resolution`] — the
//! single resolved-concept / resolved-attribute types in
//! `tonk-schema`. `Resolver` is the analyzer's vocabulary: it
//! keeps the analyzer agnostic of *where* names resolve from.

use async_trait::async_trait;
use dialog_artifacts::Entity;
use dialog_common::ConditionalSync;

use crate::concept::{QueryEnv, lookup_named_entity};
use crate::query_source::Source;
use crate::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, Environment,
    NamedReference, ResolveError,
};

/// Look up names against a backing store (typically the branch).
///
/// The analyzer calls this when it encounters a concept name in
/// head position or a bare-symbol reference in field-value
/// position. [`SourceResolver`] is the live implementation;
/// [`NoopResolver`] is the document-only one.
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

/// A [`Resolver`] backed by a live [`Source`] — a `&Branch` or a
/// `&Transaction` overlay — and the query environment its
/// operations take.
///
/// Every `resolve_*` method delegates to the [`crate::resolution`]
/// surface (`ConceptReference::resolve(...).perform(env)`, etc.).
/// This is the type the evaluator and the language server both
/// hand to [`super::analyze`] when resolving against real data.
pub struct SourceResolver<'a, Env> {
    /// The source the lookups query against.
    source: Source<'a>,
    /// The query environment the resolution chain handles take.
    env: &'a Env,
}

impl<'a, Env> SourceResolver<'a, Env> {
    /// Build a resolver bound to `source` and `env`.
    pub fn new(source: impl Into<Source<'a>>, env: &'a Env) -> Self {
        Self {
            source: source.into(),
            env,
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<Env: QueryEnv> Resolver for SourceResolver<'_, Env> {
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        ConceptReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }

    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        AttributeReference::from(NamedReference(name.to_owned()))
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        AttributeReference::from(entity.clone())
            .resolve(self.source.clone())
            .perform(self.env)
            .await
    }

    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        lookup_named_entity(name, self.source.clone(), self.env).await
    }
}

/// A [`Resolver`] backed by an [`Environment`] — the seam the
/// language server drives. An `Environment` exposes the same
/// `resolve_*` / `list_*` surface but without the lifetime-bound
/// `Source`; this adapter lets the analyzer run against any
/// host-opened environment without naming the host's types.
pub struct EnvironmentResolver<'a, E: ?Sized> {
    environment: &'a E,
}

impl<'a, E: ?Sized> EnvironmentResolver<'a, E> {
    /// Build a resolver delegating to `environment`.
    pub fn new(environment: &'a E) -> Self {
        Self { environment }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<E: Environment + ConditionalSync + ?Sized> Resolver for EnvironmentResolver<'_, E> {
    async fn resolve_concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError> {
        self.environment
            .resolve_concept(ConceptReference::from(NamedReference(name.to_owned())))
            .await
    }

    async fn resolve_attribute(
        &self,
        name: &str,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        self.environment
            .resolve_attribute(AttributeReference::from(NamedReference(name.to_owned())))
            .await
    }

    async fn resolve_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        self.environment
            .resolve_attribute(AttributeReference::from(entity.clone()))
            .await
    }

    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        self.environment.resolve_named_entity(name).await
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
