//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post};
use tokio::sync::RwLock;
use tonk_space::{Delegation, Operator, Space};

use crate::ServiceWorkerStorageBackend;

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

/// Application state containing the space, operator, and delegation.
pub struct TonkState {
    /// The space being managed
    pub space: Space<ServiceWorkerStorageBackend>,
    /// The operator identity for signing invocations
    pub operator: Operator,
    /// The delegation from space to operator
    pub delegation: Delegation,
}

/// Shared application state.
pub type AppState = Arc<RwLock<TonkState>>;

/// Root handler that returns a welcome message.
async fn root(State(_state): State<AppState>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the space, operator, and delegation as shared state.
///
/// # Arguments
/// * `space` - The space being managed
/// * `operator` - The operator identity for signing invocations
/// * `delegation` - The delegation from space to operator
pub fn api_router(
    space: Space<ServiceWorkerStorageBackend>,
    operator: Operator,
    delegation: Delegation,
) -> Router {
    let state: AppState = Arc::new(RwLock::new(TonkState {
        space,
        operator,
        delegation,
    }));
    Router::new()
        .route("/api", get(root))
        .route("/api/authorize", post(authorize))
        .route("/api/status", get(status))
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
    use crate::{ServiceWorkerStorageBackend, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tonk_space::{DelegatedSubject, Delegation, Ed25519Signer, Operator, Space};
    use tower::ServiceExt;

    /// Creates a test space with operator and delegation for testing.
    ///
    /// Uses a unique random operator for each test to avoid IndexedDB conflicts
    /// between concurrent test runs, while keeping space_did == operator.did()
    /// to match how real spaces work.
    pub async fn test_space_with_delegation()
    -> (Space<ServiceWorkerStorageBackend>, Operator, Delegation) {
        // Generate unique operator - this will be both the space owner AND the operator
        // Using the same keypair for both matches how spaces work in production
        let operator = Operator::generate();
        let space_did = operator.did().to_string();

        // Create self-delegation (operator delegates to itself)
        let delegation = Delegation::builder()
            .issuer(Ed25519Signer::from(&operator))
            .audience(*operator.did())
            .subject(DelegatedSubject::Specific(*operator.did()))
            .command(vec![])
            .try_build()
            .expect("Failed to build delegation");

        let delegation = Delegation::from(delegation);

        let backend = ServiceWorkerStorageBackend::new(&space_did).await;
        let space = Space::open(space_did, &operator, backend)
            .await
            .expect("Failed to create test space");

        (space, operator, delegation)
    }

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let (space, operator, delegation) = test_space_with_delegation().await;
        let app = api_router(space, operator, delegation);

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
