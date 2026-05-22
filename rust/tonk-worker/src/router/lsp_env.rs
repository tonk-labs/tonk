//! The worker's [`EnvProvider`] — how the language server reaches
//! the live environment.
//!
//! The language server defines the [`EnvProvider`] port and is
//! handed one per request. The worker implements it: given a
//! `(repo, branch)` pair (parsed by the language server from the
//! document URI), it acquires the reactor's session and pairs the
//! branch with the operator the session's operations take.
//!
//! [`LspEnvProvider`] wraps the worker's [`AppState`]; the LSP
//! route handler builds one per request and threads it into
//! `Server::handle_message`.

use async_trait::async_trait;
use tokio::sync::OwnedRwLockReadGuard;
use tonk_language_server::EnvProvider;
use tonk_schema::query_source::Source;
use tonk_schema::resolution::{
    AttributeDefinition, AttributeReference, ConceptDefinition, ConceptReference, Entity,
    Environment, ResolveError,
};

use crate::reactor::BranchSession;
use crate::router::AppState;
use crate::worker::TonkState;

/// The worker's [`EnvProvider`], built around the shared
/// [`AppState`]. Each LSP request gets a fresh one.
pub struct LspEnvProvider {
    state: AppState,
}

impl LspEnvProvider {
    /// Wrap the worker's state handle as an [`EnvProvider`].
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl EnvProvider for LspEnvProvider {
    type Env = LiveEnvironment;

    async fn environment(&self, repo: &str, branch: &str) -> Option<Self::Env> {
        // Hold a read guard for the request's lifetime — the
        // operator the resolution chain takes lives inside
        // `TonkState`, and resolution is read-only so concurrent
        // readers are fine.
        let guard = self.state.clone().read_owned().await;
        let reference = guard.reactor.repository(repo).branch(branch);
        let session = reference.acquire(&guard.operator).await.ok()?;
        Some(LiveEnvironment { guard, session })
    }
}

/// A live environment: the reactor session for one branch, paired
/// with the operator its queries run on. The read guard keeps the
/// operator borrowable for the whole request.
pub struct LiveEnvironment {
    guard: OwnedRwLockReadGuard<TonkState>,
    session: BranchSession,
}

impl LiveEnvironment {
    /// The branch wrapped as a resolution [`Source`].
    fn source(&self) -> Source<'_> {
        Source::from(self.session.handle())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Environment for LiveEnvironment {
    async fn resolve_concept(
        &self,
        reference: ConceptReference,
    ) -> Result<Option<ConceptDefinition>, ResolveError> {
        reference
            .resolve(self.source())
            .perform(&self.guard.operator)
            .await
    }

    async fn resolve_attribute(
        &self,
        reference: AttributeReference,
    ) -> Result<Option<AttributeDefinition>, ResolveError> {
        reference
            .resolve(self.source())
            .perform(&self.guard.operator)
            .await
    }

    async fn resolve_named_entity(&self, name: &str) -> Result<Option<Entity>, ResolveError> {
        tonk_schema::concept::lookup_named_entity(name, self.source(), &self.guard.operator).await
    }

    async fn list_concepts(&self) -> Result<Vec<ConceptDefinition>, ResolveError> {
        ConceptDefinition::list(self.source())
            .perform(&self.guard.operator)
            .await
    }

    async fn list_names(&self) -> Result<Vec<tonk_schema::meta::Name>, ResolveError> {
        tonk_schema::resolution::NamedReference::list(self.source())
            .perform(&self.guard.operator)
            .await
    }
}
