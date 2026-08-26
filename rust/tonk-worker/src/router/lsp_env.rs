//! The worker's [`EnvProvider`] — how the language server reaches
//! the live source + env pair for a document.
//!
//! The language server defines the [`EnvProvider`] port and is
//! handed one per request. The worker implements it: given a
//! `(repo, branch)` pair (parsed by the language server from the
//! document URI), it acquires the reactor's session and pairs the
//! branch (as a [`Source`]) with the operator the session's
//! operations take. A [`Repo::Profile`] repo resolves through the
//! reactor's profile handle rather than the named-repo namespace,
//! so `/inspector` (which inspects the profile DB) reaches a live
//! branch like any space does.
//!
//! [`LspEnvProvider`] wraps the worker's [`AppState`]; the LSP
//! route handler builds one per request and threads it into
//! `Server::handle_message`.

use async_trait::async_trait;
use tokio::sync::OwnedRwLockReadGuard;
use tonk_language_server::{EnvProvider, Opened, Repo};
use tonk_schema::query_source::Source;

use crate::reactor::BranchSession;
use crate::router::AppState;
use crate::worker::{DefaultOperator, TonkState};

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
    type Opened = LiveEnvironment;

    async fn open(&self, repo: &Repo, branch: &str) -> Option<Self::Opened> {
        // Hold a read guard for the request's lifetime — the
        // operator the resolution chain takes lives inside
        // `TonkState`, and resolution is read-only so concurrent
        // readers are fine.
        let guard = self.state.clone().read_owned().await;
        // The profile lives outside the named-repo namespace, so it
        // needs its own handle; both yield the same chain surface
        // from `branch` on.
        let repository = match repo {
            Repo::Profile(_) => guard.reactor.profile_repository(),
            Repo::Named(name) => guard.reactor.repository(name),
        };
        let session = repository
            .branch(branch)
            .acquire(&guard.operator)
            .await
            .ok()?;
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

impl Opened for LiveEnvironment {
    type Env = DefaultOperator;

    fn source(&self) -> Source<'_> {
        Source::from(self.session.handle())
    }

    fn env(&self) -> &Self::Env {
        &self.guard.operator
    }
}
