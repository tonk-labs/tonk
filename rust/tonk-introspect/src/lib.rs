#![warn(missing_docs)]
//! Introspection trait surface for tonk hosts and tools.
//!
//! Three trait families, scoped by what they can answer about:
//!
//! - [`SystemIntrospection`] — the host as a whole (the repos it
//!   knows about, the active profile).
//! - [`RepositoryIntrospection`] — one repository (its branches,
//!   its remotes).
//! - [`BranchIntrospection`] — one branch (concept and attribute
//!   schema, published names).
//!
//! Each trait is a *contract*, not an implementation: hosts wire
//! it up over whatever backing store they already have (a reactor,
//! an open dialog branch, an in-memory fixture for tests). Tools
//! that consume the contract — language server completions, docs
//! generators, REPL helpers — depend only on this crate, so their
//! dependency tree stays small.
//!
//! `SystemIntrospection` and `RepositoryIntrospection` are
//! intentionally sketches today; only [`BranchIntrospection`] has a
//! filled-out method set, because that's what the first consumer
//! (language server completion) needs. The other two are reserved
//! so future code can land against an existing trait shape rather
//! than inventing one ad hoc.

use async_trait::async_trait;
use dialog_artifacts::Entity;
use dialog_query::{AttributeDescriptor, ConceptDescriptor};
use thiserror::Error;

/// An attribute resolved from a branch — its entity URI plus the
/// reconstructed descriptor.
#[derive(Debug, Clone)]
pub struct ResolvedAttribute {
    /// The attribute's entity URI.
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: AttributeDescriptor,
}

/// A concept resolved from a branch — its entity URI plus the
/// reconstructed descriptor.
#[derive(Debug, Clone)]
pub struct ResolvedConcept {
    /// The concept entity URI.
    pub entity: Entity,
    /// The reconstructed descriptor.
    pub descriptor: ConceptDescriptor,
}

/// A published name — the user-facing label and the entity it
/// currently identifies.
#[derive(Debug, Clone)]
pub struct NamedEntity {
    /// The name as it appears in `id:<name>` URIs (without the
    /// `id:` prefix).
    pub name: String,
    /// The entity the name currently points at.
    pub entity: Entity,
}

/// Opaque error from an [`BranchIntrospection`] implementation.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct IntrospectionError {
    /// Human-readable description of the underlying failure.
    pub message: String,
}

impl IntrospectionError {
    /// Construct an error from any displayable value.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Branch-level introspection — concept and attribute schema,
/// published names, point lookups.
///
/// Implementors hold a handle to a single open branch and answer
/// questions about its current state. The interface deliberately
/// mixes point lookups (`lookup_*`) and enumerations (`list_*`):
/// both shapes are useful, and there's no clean way to compose
/// one out of the other without leaking the implementation's I/O
/// model.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait BranchIntrospection {
    /// Look up a concept by published name. Returns `None` when
    /// the name has no matching entity on this branch.
    async fn lookup_concept(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedConcept>, IntrospectionError>;

    /// Look up an attribute by published name.
    async fn lookup_attribute(
        &self,
        name: &str,
    ) -> Result<Option<ResolvedAttribute>, IntrospectionError>;

    /// Look up an attribute by its entity URI.
    async fn lookup_attribute_by_entity(
        &self,
        entity: &Entity,
    ) -> Result<Option<ResolvedAttribute>, IntrospectionError>;

    /// Look up the entity a published name currently points at.
    async fn lookup_named_entity(&self, name: &str) -> Result<Option<Entity>, IntrospectionError>;

    /// Enumerate every concept defined on this branch — built-in
    /// or user-published. Returns descriptors so a caller can
    /// surface fields, descriptions, etc. without a follow-up
    /// `lookup_concept` round-trip.
    async fn list_concepts(&self) -> Result<Vec<ResolvedConcept>, IntrospectionError>;

    /// Enumerate every published name on this branch and the
    /// entity each currently points at. Useful for completion
    /// surfaces that suggest reference targets in value position.
    async fn list_named_entities(&self) -> Result<Vec<NamedEntity>, IntrospectionError>;
}

/// Repository-level introspection — branches in a repo, the
/// repo's identity, its remotes.
///
/// **Stub today.** Method set will fill in as consumers appear;
/// the trait exists so future code lands against a stable name.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait RepositoryIntrospection {
    /// Convenience accessor for the repository's name as the host
    /// knows it. Distinct from the repository's intrinsic
    /// `did:key:` identity, which lives elsewhere.
    fn name(&self) -> &str;
}

/// System-level introspection — the host as a whole.
///
/// **Stub today.** Method set will fill in as consumers appear.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait SystemIntrospection {
    /// Names of every repository the host currently knows about.
    /// Order unspecified; implementations are free to return them
    /// in whatever order is cheapest.
    async fn list_repositories(&self) -> Result<Vec<String>, IntrospectionError>;
}
