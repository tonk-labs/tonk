//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post};
use tokio::sync::RwLock;

use crate::worker::TonkState;

mod authorize;
pub use authorize::{AuthorizeResponse, authorize};

mod fact;
pub use fact::{AssertResponse, FactQuery, FactResponse, QueryResponse, assert_fact, query_facts};

mod inspect;
pub use inspect::{
    BranchStatusResponse, CredentialsResponse, RemoteBranchStatusResponse, SiteStatusResponse,
    UpstreamStatusResponse, branch, site,
};

mod status;
pub use status::{StatusResponse, status};

mod sync;
pub use sync::SyncResponse;

mod identify;
pub use identify::{IdentifyResponse, identify};

mod delegations;
pub use delegations::{DelegationsResponse, delegations};

/// Shared application state containing identity and workspace.
pub type AppState = Arc<RwLock<TonkState>>;

/// Root handler that returns a welcome message.
async fn root(State(_state): State<AppState>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the TonkState as shared state.
pub fn api_router(state: TonkState) -> Router {
    let state: AppState = Arc::new(RwLock::new(state));
    Router::new()
        .route("/api", get(root))
        .route("/api/identify", get(identify))
        .route("/api/authorize", post(authorize))
        .route("/api/status", get(status))
        .route("/api/delegations", get(delegations))
        .route("/api/inspect/branch/{branch_name}", get(inspect::branch))
        .route("/api/inspect/site/{site_name}", get(inspect::site::site))
        .route(
            "/api/inspect/site/{site}/{repo_did}/branch/{branch}",
            get(inspect::site::branch),
        )
        .route(
            "/api/fact/assert/{entity}/{attribute_ns}/{attribute_name}",
            post(assert_fact),
        )
        .route("/api/fact/query", get(query_facts))
        .route("/api/sync", post(sync::sync))
        .route("/api/sync/pull", post(sync::pull))
        .route("/api/sync/push", post(sync::push))
        .with_state(state)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub mod tests {
    use std::sync::Arc;

    use crate::worker::TonkState;
    use crate::workspace::WorkspaceError;
    use crate::{Identity, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    pub async fn test_state() -> TonkState {
        let mut identity = Identity::load_or_create()
            .await
            .expect("Failed to create test identity");

        let workspace = match identity.open_workspace(None).await {
            Ok(ws) => ws,
            Err(WorkspaceError::NoDefaultSpace) => identity
                .create_workspace()
                .await
                .expect("Failed to create workspace"),
            Err(e) => panic!("Failed to open workspace: {}", e),
        };

        TonkState {
            identity: Arc::new(identity),
            workspace,
        }
    }

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let state = test_state().await;
        let app = api_router(state);

        let request = Request::builder()
            .uri("/api")
            .method("GET")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");
        assert_eq!(body.as_ref(), b"Hello, Tonk!");
    }
}
