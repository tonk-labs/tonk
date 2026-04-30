//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{Router, extract::State, routing::get, routing::post, routing::put};
use tokio::sync::RwLock;

use crate::worker::TonkState;

mod claim;
pub use claim::{AssertPath, AssertResponse, ClaimQuery, ClaimResponse, QueryResponse};

mod join;
pub use join::{JoinRequest, JoinResponse};

mod create_invite;
pub use create_invite::{CreateInviteRequest, CreateInviteResponse};

pub mod inspect;
pub use inspect::{BranchStatusResponse, RemoteBranchStatusResponse, RemoteStatusResponse};

mod repository;
pub use repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, bootstrap_profile_meta,
};

mod sync;
pub use dialog_repository::Revision;
pub use sync::SyncResponse;

mod identify;
pub use identify::IdentifyResponse;

pub mod lsp;
pub use lsp::LspHub;

mod profile;
pub use profile::ProfileInfo;

mod transact;
pub use transact::{TransactPath, TransactResponse};

mod query;
pub use query::{QueryPath, QueryResult, QueryResultEnvelope};

/// Shared application state containing profile and operator.
pub type AppState = Arc<RwLock<TonkState>>;

/// Root handler that returns a welcome message.
async fn root(State(_state): State<AppState>) -> &'static str {
    "Hello, Tonk!"
}

/// Creates the API router with all configured routes.
///
/// Sets up the routing tree with the TonkState as shared state.
/// Returns the assembled router along with the [`LspHub`] handle so
/// the SW entry point can call [`LspHub::shutdown`] when a newer
/// worker version begins installing.
pub fn api_router(state: TonkState) -> (Router, Arc<LspHub>) {
    let state: AppState = Arc::new(RwLock::new(state));
    let (lsp_routes, lsp_hub) = lsp::lsp_router();
    let router = Router::new()
        .route("/api", get(root))
        .route("/api/identify", get(identify::identify))
        .route("/api/profile", get(profile::get_profile))
        // Join an invite — creates a fresh replica or refreshes
        // access on an existing one. See `router/join.rs`.
        .route("/api/profile/join", post(join::join))
        // Repository lifecycle
        .route(
            "/api/repository/{repo}",
            put(repository::put_repository).get(repository::get_repository),
        )
        // Invite minting — see `router/create_invite.rs`. Two modes
        // (audience-open and audience-scoped) keyed off the request
        // body shape.
        .route(
            "/api/repository/{repo}/invite",
            post(create_invite::create_invite),
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
        // Transaction route — accepts a tonk-schema transaction
        // document (JSON or YAML) and commits all derived facts in
        // a single transaction.
        .route(
            "/api/repository/{repo}/branch/{branch}/transact",
            post(transact::transact),
        )
        // Query route — accepts an asserted-notation query
        // document and returns matching entities.
        .route(
            "/api/repository/{repo}/branch/{branch}/query",
            post(query::query),
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
        // LSP routes carry their own state (`Extension<LspHub>`) so
        // they don't need to know about `AppState`. Merging keeps
        // the language-server lifetime tied to the worker.
        .merge(lsp_routes);
    (router, lsp_hub)
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
    use dialog_credentials::Ed25519Signer;
    use dialog_operator::Profile;
    use dialog_storage::provider::storage::Storage;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject as UcanSubject};
    use dialog_varsig::Principal as _;
    use tonk_invite::{Invite, InviteAudience};

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

        TonkState {
            profile,
            operator,
            profile_name: "test-tonk".to_string(),
        }
    }

    /// Creates a test repository via `PUT /api/repository/{name}`.
    ///
    /// Each test calls this with its own repo name so runs are
    /// independent. Tolerates `412 Precondition Failed` in case the
    /// same name was used by a prior run within the same browser
    /// session (IndexedDB state survives).
    async fn put_repo(app: &Router, name: &str) {
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

    #[dialog_common::test]
    async fn it_responds_to_root_api_request() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

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
        let (app, _lsp) = api_router(state);

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
        let (app, _lsp) = api_router(state);
        let repo = "test-create";

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
        let (app, _lsp) = api_router(state);
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
        let (app, _lsp) = api_router(state);
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
    async fn it_routes_invite_minting() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-invite-route";

        // Create the repo first so the invite handler can load it.
        put_repo(&app, repo).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/invite", repo))
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Whatever the handler decides to do, we want a 2xx — proving
        // the route is reachable. A 404 here would mean the route
        // failed to match, which is the exact regression we're
        // guarding against.
        let status = response.status();
        assert!(
            status.is_success(),
            "expected 2xx from POST /api/repository/{}/invite, got {}",
            repo,
            status,
        );
    }

    /// Synthesize an audience-open invite for a fresh subject the
    /// caller's profile has never seen. Returns the URL plus the
    /// subject DID so tests can assert against it.
    ///
    /// Builds the invite directly in test code (no endpoint round
    /// trip): generate a random subject keypair and a random
    /// ephemeral keypair, build a one-hop chain `subject ->
    /// ephemeral` scoped to the subject, and serialize as an
    /// audience-open invite. Recipient extends the chain by one
    /// hop in [`super::join::join`].
    async fn synthesize_open_invite() -> (String, dialog_varsig::Did) {
        let subject_signer = Ed25519Signer::generate().await.unwrap();
        let subject_did = subject_signer.did();

        let ephemeral_seed: [u8; 32] = rand::random();
        let ephemeral = Ed25519Signer::import(&ephemeral_seed).await.unwrap();
        let ephemeral_did = ephemeral.did();

        let delegation = DelegationBuilder::new()
            .issuer(subject_signer)
            .audience(&ephemeral_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(delegation);

        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: ephemeral_seed,
            },
            None,
        )
        .await
        .unwrap();
        let url = invite.to_url(tonk_invite::DEFAULT_BASE_URL).unwrap();
        (url, subject_did)
    }

    /// Issue `POST /api/profile/join` and return status + parsed
    /// JSON body (or raw bytes when JSON parsing fails).
    async fn post_join(app: &Router, url: &str, name: &str) -> (StatusCode, serde_json::Value) {
        let body = serde_json::json!({ "url": url, "name": name }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/profile/join")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[dialog_common::test]
    async fn it_joins_a_fresh_invite_with_joined_outcome() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let (invite_url, subject_did) = synthesize_open_invite().await;
        let (status, body) = post_join(&app, &invite_url, "fresh-join").await;

        assert_eq!(
            status,
            StatusCode::CREATED,
            "expected 201 Created on first join, got {status}: {body}",
        );
        assert_eq!(body["outcome"], "joined", "expected joined outcome: {body}");
        assert_eq!(body["repository"]["name"], "fresh-join");
        assert_eq!(body["repository"]["subject"], subject_did.to_string());
    }

    #[dialog_common::test]
    async fn it_renews_when_subject_already_mounted() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let (invite_url, _subject_did) = synthesize_open_invite().await;

        // First join creates the replica under "renew-original".
        let (first_status, first_body) = post_join(&app, &invite_url, "renew-original").await;
        assert_eq!(
            first_status,
            StatusCode::CREATED,
            "first join: {first_body}"
        );

        // Second join of the *same invite URL* — same subject, the
        // recipient already has it mounted. Worker should respond
        // with a `renewed` outcome and return the existing replica
        // ("renew-original"), regardless of the requested name.
        let (second_status, second_body) = post_join(&app, &invite_url, "different-name").await;
        assert_eq!(
            second_status,
            StatusCode::OK,
            "expected 200 OK on second join: {second_body}",
        );
        assert_eq!(
            second_body["outcome"], "renewed",
            "expected renewed outcome: {second_body}",
        );
        assert_eq!(
            second_body["repository"]["name"], "renew-original",
            "renewed should return the existing replica name, not the requested one",
        );
    }

    #[dialog_common::test]
    async fn it_rejects_unrelated_name_collision_on_join() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        // Pre-existing space named "claimed-name" — created via
        // a regular PUT, with a different subject than the invite
        // we'll try to redeem.
        put_repo(&app, "claimed-name").await;

        let (invite_url, _subject_did) = synthesize_open_invite().await;
        let (status, body) = post_join(&app, &invite_url, "claimed-name").await;

        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "expected 409 Conflict for name collision, got {status}: {body}",
        );
    }

    #[dialog_common::test]
    async fn it_rejects_malformed_invite_url() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let (status, _body) = post_join(&app, "not-a-url", "doesnt-matter").await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "expected 400 Bad Request for malformed invite",
        );
    }

    #[dialog_common::test]
    async fn it_returns_repository_info() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
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
        let (app, _lsp) = api_router(state);
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
        let (app, _lsp) = api_router(state);
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
    async fn it_inspects_branch_after_commit() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
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

    /// Reproduces the editor's reported failure on an
    /// attribute+attribute+concept document submitted as a
    /// single YAML body. The new analyzer should accept all
    /// three expressions, build the concept's descriptor from
    /// the in-document attribute definitions, and commit
    /// everything in one transaction.
    #[dialog_common::test]
    async fn it_transacts_attributes_and_concept_in_one_doc() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-multi";

        put_repo(&app, repo).await;

        let body = "\
attribute! person-name:
    the:         io.gozala.person/name
    as:          Text
    cardinality: one
    description: The person's name


attribute! person-age:
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one
  description: The person's age

concept! person:
    description: A person
    with:
      name: person-name
      age:  person-age
";

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "expected 200 OK; got {status}: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        let resp: super::TransactResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(resp.claims > 0);
        // The combined plan ends on the concept head; the
        // response surfaces only the last head's entity (single
        // entry in the entities map for now).
        assert_eq!(resp.entities.len(), 1);
        let (label, entity) = resp.entities.iter().next().unwrap();
        assert_eq!(label, "concept");
        assert!(entity.starts_with("concept:"));
    }

    /// A single `attribute!` assertion commits cleanly via the
    /// new YAML notation.
    #[dialog_common::test]
    async fn it_transacts_a_single_attribute() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-single";

        put_repo(&app, repo).await;

        let body = "\
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: The person's name
";

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "expected 200 OK; got {status}: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        let resp: super::TransactResponse = serde_json::from_slice(&body_bytes).unwrap();
        let entity = resp.entities.get("attribute").unwrap();
        assert!(entity.starts_with("the:"));
    }

    /// Sending a parser-rejected document returns 400 with a
    /// human-readable error.
    #[dialog_common::test]
    async fn it_returns_400_on_malformed_transaction() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-malformed";

        put_repo(&app, repo).await;

        // YAML with an indentation error — saphyr rejects it.
        let body = "person!:\n  name: Alice\n bad-indent: 1\n";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// A concept's `with` block can address an attribute defined
    /// in a *prior* transaction by bookmark name. The
    /// branch-backed resolver looks up the previously-asserted
    /// attribute by `dialog.meta/name`, reconstructs its
    /// descriptor, and lets the concept hash correctly.
    #[dialog_common::test]
    async fn it_resolves_bookmarks_across_transactions() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-cross";

        put_repo(&app, repo).await;

        // First transaction: define `person-name` only.
        let first = "\
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: The person's name
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(first))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Second transaction: define `person-age` and a concept
        // `person` whose `with.name` references the *previously*
        // declared `person-name`.
        let second = "\
attribute! person-age:
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one

concept! person:
  description: A person
  with:
    name: person-name
    age:  person-age
";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(second))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "expected 200 OK; got {status}: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        let resp: super::TransactResponse = serde_json::from_slice(&body_bytes).unwrap();
        let entity = resp.entities.get("concept").unwrap();
        assert!(entity.starts_with("concept:"));
    }
}
