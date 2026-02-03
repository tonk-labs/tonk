//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post};
use tokio::sync::RwLock;

use crate::worker::TonkState;

mod authorize;
pub use authorize::{AuthorizeRequest, AuthorizeResponse, authorize};

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

/// Shared application state containing identity and session.
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
            "/api/inspect/site/{site}/{repo_did}/archive/index/{hash}",
            get(inspect::site::archive_block),
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

/// Test utilities for router tests.
#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use crate::worker::TonkState;
    use crate::{Identity, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Creates a test state with identity and session for testing routes.
    pub async fn test_state() -> TonkState {
        let mut identity = Identity::load_or_create()
            .await
            .expect("Failed to create test identity");

        // Get known spaces, or create if none exist
        let known_spaces = identity
            .account()
            .known_spaces()
            .await
            .expect("Could not query known spaces");

        let session = if let Some(space_did) = known_spaces.first() {
            identity
                .open_session(space_did)
                .await
                .expect("Failed to open session")
        } else {
            identity
                .create_session()
                .await
                .expect("Failed to create session")
        };

        TonkState {
            identity: Arc::new(identity),
            session,
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
