//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post};
use tokio::sync::RwLock;

use crate::worker::TonkState;

mod claim;
pub use claim::{AssertPath, AssertResponse, ClaimQuery, ClaimResponse, QueryResponse};

mod init;
pub use init::InitResponse;

pub mod inspect;
pub use inspect::{BranchStatusResponse, RemoteBranchStatusResponse, RemoteStatusResponse};

mod status;
pub use status::StatusResponse;

mod sync;
pub use sync::SyncResponse;

mod identify;
pub use identify::IdentifyResponse;

/// Shared application state containing profile and operator.
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
        .route("/api/identify", get(identify::identify))
        // Repository status
        .route("/api/repository/{repo}/status", get(status::status))
        // Branch init (set up UCAN remote + upstream)
        .route(
            "/api/repository/{repo}/branch/{branch}/init",
            post(init::init),
        )
        // Sync operations
        .route(
            "/api/repository/{repo}/branch/{branch}/sync",
            post(sync::sync),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/sync/pull",
            post(sync::pull),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/sync/push",
            post(sync::push),
        )
        // Claim operations
        .route(
            "/api/repository/{repo}/branch/{branch}/claim/assert/{entity}/{attr_ns}/{attr_name}",
            post(claim::assert_claim),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/claim/retract/{entity}/{attr_ns}/{attr_name}",
            post(claim::retract_claim),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/claim/select",
            get(claim::select_claims),
        )
        // Inspect operations
        .route(
            "/api/inspect/repository/{repo}/branch/{branch}",
            get(inspect::branch::inspect_branch),
        )
        .route(
            "/api/inspect/repository/{repo}/remote/{remote}",
            get(inspect::remote::inspect_remote),
        )
        .route(
            "/api/inspect/repository/{repo}/remote/{remote}/branch/{branch}",
            get(inspect::remote::inspect_remote_branch),
        )
        .route(
            "/api/inspect/repository/{repo}/archive/index/{hash}",
            get(inspect::archive::inspect_archive_block),
        )
        .route(
            "/api/inspect/repository/{repo}/remote/{remote}/archive/index/{hash}",
            get(inspect::archive::inspect_remote_archive_block),
        )
        .with_state(state)
}

/// Test utilities for router tests.
///
/// These tests run in a WASM service worker context since TonkState
/// requires IndexedDB (WASM) or filesystem (native) storage.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use crate::api_router;
    use crate::worker::TonkState;

    use dialog_capability::Subject;
    use dialog_repository::RepositoryExt as _;
    use dialog_repository::profile::Profile;
    use dialog_storage::provider::storage::Storage;

    use crate::worker::DefaultSpace;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Creates a test state with the default storage backend.
    pub async fn test_state() -> TonkState {
        crate::patch_idb_versionchange();
        let storage = Storage::<DefaultSpace>::default();
        let profile = Profile::open("test-tonk")
            .perform(&storage)
            .await
            .expect("Failed to create test profile");

        let operator = profile
            .derive(b"test-worker")
            .allow(Subject::any())
            .build(storage)
            .await
            .expect("Failed to build test operator");

        // Open default repo
        let repo = profile
            .repository("home")
            .open()
            .perform(&operator)
            .await
            .expect("Failed to open test repo");

        // Delegate repo access to profile
        if let Some(access) = repo.try_access() {
            if let Ok(chain) = access
                .claim(&repo)
                .delegate(profile.did())
                .perform(&operator)
                .await
            {
                let _ = profile.access().save(chain).perform(&operator).await;
            }
        }

        TonkState { profile, operator }
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

    #[dialog_common::test]
    async fn it_returns_identify() {
        let state = test_state().await;
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/identify")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::IdentifyResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.did.starts_with("did:key:"));
    }

    #[dialog_common::test]
    async fn it_returns_status() {
        let state = test_state().await;
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::StatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.repo_name, "home");
        assert!(!resp.space_did.is_empty());
        assert!(!resp.has_upstream);
    }

    #[dialog_common::test]
    async fn it_asserts_and_selects_claims() {
        let state = test_state().await;
        let app = api_router(state);

        // Assert a fact
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/claim/assert/test:entity/test/name")
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("Test Name"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Query the fact
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/claim/select?the=test/name&of=test:entity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::QueryResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.claims.len(), 1);
        assert_eq!(resp.claims[0].is, serde_json::json!("Test Name"));
    }

    #[dialog_common::test]
    async fn it_initializes_branch() {
        let state = test_state().await;
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/init")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::InitResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.success);
    }

    #[dialog_common::test]
    async fn it_syncs_after_commit() {
        let state = test_state().await;
        let app = api_router(state);

        // First assert a fact so the branch has data
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/claim/assert/test:sync/test/value")
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("sync test"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Now sync — without upstream it should return OK with no changes
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/sync")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[dialog_common::test]
    async fn it_inspects_branch_after_commit() {
        let state = test_state().await;
        let app = api_router(state);

        // Commit some data first so the branch exists
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/home/branch/main/claim/assert/test:inspect/test/value")
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("inspect test"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Now inspect the branch
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/inspect/repository/home/branch/main")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
