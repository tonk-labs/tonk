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

mod space_list;
pub use space_list::{ListSpacesResponse, SpaceInfo, list_spaces};

mod space_create;
pub use space_create::{CreateSpaceRequest, CreateSpaceResponse, create_space};

mod space_metadata;
pub use space_metadata::{
    SpaceMetadataResponse, UpdateMetadataRequest, get_metadata, update_metadata,
};

/// Shared application state containing identity and session.
pub type AppState = Arc<RwLock<TonkState>>;

/// Root handler that returns a welcome message.
async fn root(State(_state): State<AppState>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the TonkState as shared state.
/// Routes are organized into:
/// - Global endpoints: `/api/*` - identity-level operations
/// - Space endpoints: `/api/{multikey}/*` - space-specific operations
///
/// The `multikey` is the `z6Mk...` portion of a DID (`did:key:z6Mk...`).
pub fn api_router(state: TonkState) -> Router {
    let state: AppState = Arc::new(RwLock::new(state));
    Router::new()
        // Global endpoints (no space context required)
        .route("/api", get(root))
        .route("/api/identify", get(identify))
        .route("/api/space/list", get(list_spaces))
        .route("/api/space/create", post(create_space))
        // Space-specific endpoints - prefixed with multikey
        .route("/api/{multikey}/authorize", post(authorize))
        .route("/api/{multikey}/status", get(status))
        .route("/api/{multikey}/delegations", get(delegations))
        .route(
            "/api/{multikey}/metadata",
            get(get_metadata).put(update_metadata),
        )
        .route(
            "/api/{multikey}/inspect/branch/{branch_name}",
            get(inspect::branch),
        )
        .route(
            "/api/{multikey}/inspect/site/{site_name}",
            get(inspect::site::site),
        )
        .route(
            "/api/{multikey}/inspect/site/{site}/{repo_did}/branch/{branch}",
            get(inspect::site::branch),
        )
        .route(
            "/api/{multikey}/inspect/site/{site}/{repo_did}/archive/index/{hash}",
            get(inspect::site::archive_block),
        )
        .route(
            "/api/{multikey}/fact/assert/{entity}/{attribute_ns}/{attribute_name}",
            post(assert_fact),
        )
        .route("/api/{multikey}/fact/query", get(query_facts))
        .route("/api/{multikey}/sync", post(sync::sync))
        .route("/api/{multikey}/sync/pull", post(sync::pull))
        .route("/api/{multikey}/sync/push", post(sync::push))
        .with_state(state)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::RwLock;

    use crate::worker::TonkState;
    use crate::{Identity, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Creates a test state with a single space.
    /// Returns the state and the multikey of the test space.
    pub async fn test_state() -> (TonkState, String) {
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

        let space_did = session.space_did().to_string();
        let multikey = space_did
            .strip_prefix("did:key:")
            .unwrap_or(&space_did)
            .to_string();

        // Pre-cache the session
        let mut sessions = HashMap::new();
        sessions.insert(space_did, session);

        let state = TonkState {
            identity: Arc::new(RwLock::new(identity)),
            sessions: Arc::new(RwLock::new(sessions)),
        };

        (state, multikey)
    }

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let (state, _multikey) = test_state().await;
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
