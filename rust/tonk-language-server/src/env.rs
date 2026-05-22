//! The environment seam — the port the language server defines
//! and the host implements.
//!
//! The language server resolves diagnostics, completion, and
//! hover against the *live environment* the document belongs to.
//! It must not know how that environment is opened — that is the
//! host's job (a reactor session in the worker, nothing at all in
//! a standalone editor). So the language server defines the port;
//! the host implements it; the dependency points host → language
//! server.
//!
//! [`EnvProvider`] is passed to [`crate::Server::handle_message`]
//! **per request**, never stored. Each message carries its own
//! environment, the same way every worker route handler receives
//! the host state. The no-host case uses [`NoEnv`], whose
//! `environment` always returns `None`.

use async_trait::async_trait;
use dialog_common::ConditionalSync;
use tonk_schema::meta::Name;
use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, Entity,
    Environment, ResolveError,
};

/// The port the host implements so the language server can open
/// the live environment for a document.
///
/// The language server parses a document URI to `(repo, branch)`
/// and calls [`environment`](Self::environment) to acquire the
/// matching [`Environment`] — the live source resolution runs
/// against. Returning `None` is fine: the language server then
/// sees only the document's own declarations.
///
/// `?Send` on wasm because the host's environment often borrows a
/// non-`Send` reactor session.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait EnvProvider {
    /// The live environment the host opens — what resolution runs
    /// against. `ConditionalSync` so the analyzer's `analyze`
    /// (which bounds its resolver `Send + Sync` on native) accepts
    /// a resolver built over it.
    type Env: Environment + ConditionalSync;

    /// Open the live environment for `(repo, branch)`, or `None`
    /// when the host knows no such branch.
    async fn environment(&self, repo: &str, branch: &str) -> Option<Self::Env>;
}

/// An [`EnvProvider`] with no host behind it — `environment`
/// always returns `None`. Tests and a standalone editor pass this
/// so the language server resolves only the document's own
/// declarations.
pub struct NoEnv;

/// The never-constructed environment [`NoEnv`] would return.
///
/// `NoEnv::environment` is hardwired to `None`, so no value of
/// this type is ever produced; the [`Environment`] impl exists
/// only to satisfy the associated-type bound. Every method is
/// unreachable — `match *self {}` discharges the uninhabited
/// type.
pub enum NoEnvironment {}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Environment for NoEnvironment {
    async fn resolve_concept(
        &self,
        _reference: ConceptReference,
    ) -> Result<Option<ConceptDefinition>, ResolveError> {
        match *self {}
    }

    async fn resolve_attribute(
        &self,
        _reference: AttributeReference,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        match *self {}
    }

    async fn resolve_named_entity(&self, _name: &str) -> Result<Option<Entity>, ResolveError> {
        match *self {}
    }

    async fn list_concepts(&self) -> Result<Vec<ConceptDefinition>, ResolveError> {
        match *self {}
    }

    async fn list_names(&self) -> Result<Vec<Name>, ResolveError> {
        match *self {}
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl EnvProvider for NoEnv {
    type Env = NoEnvironment;

    async fn environment(&self, _repo: &str, _branch: &str) -> Option<Self::Env> {
        None
    }
}
