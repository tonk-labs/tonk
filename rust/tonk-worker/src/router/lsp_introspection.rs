//! Factory the LSP uses to acquire a branch-bound
//! [`tonk_introspect::BranchIntrospection`] for a document URI.
//!
//! Document URIs follow the
//! `tonk-buffer:///<repo>/<branch>/<cell-suffix>` shape (set by
//! `<tonk-code source>` in the editor). The factory parses
//! `(repo, branch)`, acquires a session via the reactor, and
//! hands back a `BranchResolver` wrapped as
//! `BranchIntrospection`.
//!
//! Returning `None` when the URI doesn't parse or the branch
//! can't be acquired is fine — completion gracefully degrades to
//! built-ins and document-local sources.

use std::sync::Arc;

use async_trait::async_trait;
use lsp_types::Uri;
use tonk_introspect::BranchIntrospection;
use tonk_language_server::IntrospectionFactory;

use crate::router::AppState;

/// Built off the worker's [`AppState`]; held by the LSP via
/// [`tonk_language_server::Server::with_introspection`].
pub struct ReactorIntrospectionFactory {
    state: AppState,
}

impl ReactorIntrospectionFactory {
    /// Construct a factory bound to `state` — every completion
    /// request reaches the live reactor through it.
    pub fn new(state: AppState) -> Arc<Self> {
        Arc::new(Self { state })
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl IntrospectionFactory for ReactorIntrospectionFactory {
    async fn for_uri(&self, uri: &Uri) -> Option<Arc<dyn BranchIntrospection + Send + Sync>> {
        let (repo, branch) = parse_repo_branch(uri)?;
        Some(Arc::new(LiveBranchIntrospection {
            state: self.state.clone(),
            repo,
            branch,
        }))
    }
}

/// [`BranchIntrospection`] that holds an `AppState` handle and
/// the `(repo, branch)` it was minted for. Each method
/// re-acquires the reactor session through a fresh read lock —
/// `dialog_operator::Operator` isn't `Clone`, so we can't park
/// a borrowed env in the struct.
///
/// Re-acquisition is cheap: the reactor caches branches by name
/// and returns the same `Arc<BranchState>` on the fast path.
struct LiveBranchIntrospection {
    state: AppState,
    repo: String,
    branch: String,
}

impl LiveBranchIntrospection {
    /// Acquire the reactor's session for `(self.repo,
    /// self.branch)`. Returns `None` when the branch is gone.
    async fn session(&self) -> Option<crate::reactor::BranchSession> {
        let tonk_state = self.state.read().await;
        let reference = tonk_state
            .reactor
            .repository(&self.repo)
            .branch(&self.branch);
        reference.acquire(&tonk_state.operator).await.ok()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BranchIntrospection for LiveBranchIntrospection {
    async fn lookup_concept(
        &self,
        name: &str,
    ) -> Result<Option<tonk_introspect::ResolvedConcept>, tonk_introspect::IntrospectionError> {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(None),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.lookup_concept(name).await
    }

    async fn lookup_attribute(
        &self,
        name: &str,
    ) -> Result<Option<tonk_introspect::ResolvedAttribute>, tonk_introspect::IntrospectionError>
    {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(None),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.lookup_attribute(name).await
    }

    async fn lookup_attribute_by_entity(
        &self,
        entity: &dialog_artifacts::Entity,
    ) -> Result<Option<tonk_introspect::ResolvedAttribute>, tonk_introspect::IntrospectionError>
    {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(None),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.lookup_attribute_by_entity(entity).await
    }

    async fn lookup_named_entity(
        &self,
        name: &str,
    ) -> Result<Option<dialog_artifacts::Entity>, tonk_introspect::IntrospectionError> {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(None),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.lookup_named_entity(name).await
    }

    async fn list_concepts(
        &self,
    ) -> Result<Vec<tonk_introspect::ResolvedConcept>, tonk_introspect::IntrospectionError> {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.list_concepts().await
    }

    async fn list_named_entities(
        &self,
    ) -> Result<Vec<tonk_introspect::NamedEntity>, tonk_introspect::IntrospectionError> {
        let session = match self.session().await {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let tonk_state = self.state.read().await;
        let resolver = tonk_schema::evaluate::BranchResolver {
            branch: session.handle(),
            env: &tonk_state.operator,
        };
        resolver.list_named_entities().await
    }
}

/// Pull `(repo, branch)` out of a `tonk-buffer:///<repo>/<branch>/<cell-suffix>`
/// URI. Returns `None` for any other shape — including profile
/// buffers (which use `<profile>` as the repo segment, served
/// from a different reactor accessor that we don't yet expose
/// through the factory).
fn parse_repo_branch(uri: &Uri) -> Option<(String, String)> {
    let s = uri.as_str();
    let rest = s.strip_prefix("tonk-buffer:///")?;
    // First segment is repo (or `<profile>`); second is branch.
    let mut parts = rest.splitn(3, '/');
    let repo = parts.next()?;
    let branch = parts.next()?;
    if repo.is_empty() || branch.is_empty() {
        return None;
    }
    // Profile buffers route through `profile_repository()` not
    // `repository(name)`; surface them once the LSP grows a
    // separate route. For now, fall through.
    if repo == "<profile>" {
        return None;
    }
    Some((repo.to_owned(), branch.to_owned()))
}
