//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post, routing::put};
use tokio::sync::RwLock;

use crate::worker::TonkState;

mod claim;
pub use claim::{AssertPath, AssertResponse, ClaimQuery, ClaimResponse, QueryResponse};

mod claim_invite;
pub use claim_invite::ClaimRequest;

mod create_invite;
pub use create_invite::{CreateInviteRequest, CreateInviteResponse};

mod home;

pub mod inspect;
pub use inspect::{BranchStatusResponse, RemoteBranchStatusResponse, RemoteStatusResponse};

mod repositories;
pub use repositories::ListRepositoriesResponse;

mod repository;
pub use repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration,
};

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
        // Invite claim (redeem an invite URL)
        .route("/api/claim", post(claim_invite::claim_invite))
        // Invite mint (issue a new invite URL for a repo)
        .route(
            "/api/repository/{repo}/invite",
            post(create_invite::create_invite),
        )
        // Repository list (drives the sidebar)
        .route("/api/repositories", get(repositories::list_repositories))
        // Repository lifecycle
        .route(
            "/api/repository/{repo}",
            put(repository::put_repository).get(repository::get_repository),
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
    use dialog_operator::Profile;
    use dialog_storage::provider::storage::Storage;

    use crate::worker::DefaultSpace;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Creates a test state with the default storage backend.
    ///
    /// The state has a profile and operator but *no* repository —
    /// tests that need one call [`put_repo`] with a name that is
    /// unique across the suite. IndexedDB persists across the
    /// single-process wasm test run and isn't partitioned by
    /// profile for space names, so shared repo names would cause
    /// order-dependent 201-vs-412 flips between tests.
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

        TonkState { profile, operator }
    }

    /// Issues `PUT /api/repository/{name}` and asserts 201 or 412.
    ///
    /// Tolerates `412 Precondition Failed` in case the same name was
    /// used by a prior run within the same browser session
    /// (IndexedDB state survives).
    async fn put_repo_raw(app: &Router, name: &str) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", name))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("if-none-match", "*")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::CREATED || status == StatusCode::PRECONDITION_FAILED,
            "expected 201 or 412 from PUT /api/repository/{}, got {}",
            name,
            status,
        );
    }

    /// Bootstraps the `home` meta-index before any user-facing PUT.
    ///
    /// `put_repository` registers every created repo in `home` via
    /// `home::register_repo`, which loads the home space. Tests that
    /// skip the bootstrap hit a 500 ("home repo not opened") on
    /// every non-home PUT, so all non-home test helpers call this
    /// first. In production this happens implicitly via
    /// `TonkShell::init`.
    async fn ensure_home(app: &Router) {
        put_repo_raw(app, "home").await;
    }

    /// Creates a test repository via `PUT /api/repository/{name}`,
    /// bootstrapping `home` first if needed.
    async fn put_repo(app: &Router, name: &str) {
        if name != "home" {
            ensure_home(app).await;
        }
        put_repo_raw(app, name).await;
    }

    /// POSTs `body` to `/api/repository/{repo}/invite` and returns the
    /// response status plus raw body bytes.
    async fn post_invite(app: &Router, repo: &str, body: Body) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/invite", repo))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
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
    async fn it_creates_repository() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-create";

        // `put_repository` registers into `home` on success, so home
        // must be bootstrapped first.
        ensure_home(&app).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", repo))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("if-none-match", "*")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Accept 412 on reruns — IndexedDB persists across wasm test sessions.
        let status = response.status();
        assert!(
            status == StatusCode::CREATED || status == StatusCode::PRECONDITION_FAILED,
            "expected 201 or 412, got {}",
            status,
        );
        if status == StatusCode::CREATED {
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let resp: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
            assert_eq!(resp.name, repo);
            assert!(!resp.subject.as_str().is_empty());
        }
    }

    #[dialog_common::test]
    async fn it_returns_precondition_failed_when_repo_exists() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-precondition";

        put_repo(&app, repo).await;

        // Second PUT with If-None-Match: * should fail with 412.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", repo))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("if-none-match", "*")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[dialog_common::test]
    async fn it_returns_conflict_when_repo_exists_without_precondition() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-conflict";

        put_repo(&app, repo).await;

        // Second PUT without If-None-Match should fail with 409.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", repo))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[dialog_common::test]
    async fn it_returns_repository_info() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-info";

        put_repo(&app, repo).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", repo))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.name, repo);
        assert!(!resp.subject.as_str().is_empty());
    }

    #[dialog_common::test]
    async fn it_asserts_and_selects_claims() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-claims";

        put_repo(&app, repo).await;

        // Assert a fact
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/claim/assert/test:entity/test/name",
                        repo
                    ))
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
                    .uri(format!(
                        "/api/repository/{}/branch/main/claim/select?the=test/name&of=test:entity",
                        repo
                    ))
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
    async fn it_syncs_after_commit() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-sync";

        put_repo(&app, repo).await;

        // First assert a fact so the branch has data
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/claim/assert/test:sync/test/value",
                        repo
                    ))
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
                    .uri(format!("/api/repository/{}/branch/main/sync", repo))
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[dialog_common::test]
    async fn it_mints_an_open_invite_via_create_invite_endpoint() {
        use url::Url;

        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-invite-open";

        put_repo(&app, repo).await;

        let (status, body) = post_invite(&app, repo, Body::from("{}")).await;
        assert_eq!(status, StatusCode::OK);

        let resp: super::CreateInviteResponse = serde_json::from_slice(&body).unwrap();
        assert!(
            matches!(resp, super::CreateInviteResponse::Open { .. }),
            "empty body should mint Open, got {resp:?}"
        );
        let url = resp.url();
        assert!(
            url.fragment().is_some(),
            "open invite URL must have a fragment, got {url}"
        );
        let default = Url::parse(tonk_invite::DEFAULT_BASE_URL).unwrap();
        assert_eq!(url.host_str(), default.host_str());
        assert_eq!(url.path(), default.path());
    }

    #[dialog_common::test]
    async fn it_mints_a_scoped_invite_and_omits_fragment() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal;

        const AUDIENCE_SEED: [u8; 32] = [42u8; 32];

        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-invite-scoped";

        put_repo(&app, repo).await;

        let audience_did = Ed25519Signer::import(&AUDIENCE_SEED).await.unwrap().did();
        let body = serde_json::json!({ "audience": audience_did.to_string() });
        let (status, body) =
            post_invite(&app, repo, Body::from(serde_json::to_vec(&body).unwrap())).await;
        assert_eq!(status, StatusCode::OK);

        let resp: super::CreateInviteResponse = serde_json::from_slice(&body).unwrap();
        match resp {
            super::CreateInviteResponse::Scoped { url, audience } => {
                assert_eq!(audience, audience_did);
                assert!(
                    url.fragment().is_none(),
                    "scoped invite URL must not carry a fragment, got {url}"
                );
            }
            super::CreateInviteResponse::Open { .. } => {
                panic!("explicit audience should produce Scoped, got Open")
            }
        }
    }

    #[dialog_common::test]
    async fn it_returns_404_when_minting_for_unknown_repo() {
        let state = test_state().await;
        let app = api_router(state);

        ensure_home(&app).await;

        let (status, _) = post_invite(&app, "does-not-exist", Body::from("{}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[dialog_common::test]
    async fn it_returns_400_for_malformed_invite_body() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-invite-bad-body";

        put_repo(&app, repo).await;

        let (status, _) = post_invite(&app, repo, Body::from("{not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[dialog_common::test]
    async fn it_echoes_base_url_into_minted_url() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-invite-base-url";

        put_repo(&app, repo).await;

        let body = serde_json::json!({ "base_url": "https://example.test/custom" });
        let (status, body) =
            post_invite(&app, repo, Body::from(serde_json::to_vec(&body).unwrap())).await;
        assert_eq!(status, StatusCode::OK);

        let resp: super::CreateInviteResponse = serde_json::from_slice(&body).unwrap();
        assert!(
            resp.url()
                .as_str()
                .starts_with("https://example.test/custom?access="),
            "expected minted URL to start with the provided base_url, got {}",
            resp.url(),
        );
    }

    #[dialog_common::test]
    async fn it_inspects_branch_after_commit() {
        let state = test_state().await;
        let app = api_router(state);
        let repo = "test-inspect";

        put_repo(&app, repo).await;

        // Commit some data first so the branch exists
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/claim/assert/test:inspect/test/value",
                        repo
                    ))
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
                    .uri(format!("/api/inspect/repository/{}/branch/main", repo))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
