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
pub use sync::{
    SyncResponse, SyncStatusResponse, branches_to_sync, repo_from_sync_tag, sync_repository,
};
// Re-exported so API consumers (the UI) can name the state without
// depending on `tonk-schema` directly.
pub use tonk_schema::SyncState;

mod identify;
pub use identify::IdentifyResponse;

pub mod lsp;
pub use lsp::LspHub;

mod lsp_env;

mod profile;
pub use profile::{ProfileInfo, SpaceEntry};

mod evaluate;
pub use evaluate::{CommitSummary, EvaluatePath, EvaluateResponse, QueryMatchBlock, QueryResult};

mod query;
pub use query::QueryPath;

mod transact;
pub use transact::{ProfileTransactPath, TransactPath, TransactResponse};

mod transfer;
pub use transfer::ImportResponse;

pub mod bridge;
pub use bridge::BridgeRegistry;

mod host;
pub use host::{ClientId, ViewBinding, ViewBindings};

mod migration;

mod command;
pub use command::{CommandEnv, command_registry, dispatch};

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
    api_router_from_state(Arc::new(RwLock::new(state)))
}

/// Variant of [`api_router`] that also surfaces the wrapped
/// [`AppState`] handle. The worker uses this so it can consult
/// shared state (notably the guest-binding map) outside the
/// request path, before deciding whether to dispatch into the
/// router or pass a fetch through to the network.
pub fn api_router_with_state(state: TonkState) -> (Router, AppState, Arc<LspHub>) {
    let state = Arc::new(RwLock::new(state));
    let (router, hub) = api_router_from_state(state.clone());
    (router, state, hub)
}

/// Same as [`api_router`] but takes an already-wrapped
/// [`AppState`]. Useful in tests that need to keep an `Arc`
/// handle to the state for poking the reactor directly.
pub fn api_router_from_state(state: AppState) -> (Router, Arc<LspHub>) {
    let (lsp_routes, lsp_hub) = lsp::lsp_router(state.clone());
    let router = Router::new()
        .route("/api", get(root))
        .route("/api/identify", get(identify::identify))
        .route("/api/profile", get(profile::get_profile))
        // Profile-as-repository routes. The profile is its own
        // repository but lives outside the named-repo namespace
        // (no `repo` segment), so it gets a parallel route
        // surface here rather than nesting under
        // `/api/repository/{repo}/...`.
        .route(
            "/api/profile/repository",
            get(repository::get_profile_repository),
        )
        .route(
            "/api/profile/branch/{branch}/query",
            post(query::query_profile),
        )
        .route(
            "/api/profile/branch/{branch}/evaluate",
            post(evaluate::evaluate_profile),
        )
        .route(
            "/api/profile/branch/{branch}/transact",
            post(transact::transact_profile),
        )
        // Join an invite — creates a fresh replica or refreshes
        // access on an existing one. See `router/join.rs`.
        .route("/api/profile/join", post(join::join))
        .route(
            "/api/migrate/repo-vs-profile",
            get(migration::repo_vs_profile),
        )
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
        // Opt-in remote attach — wires a remote (and branch upstream)
        // onto an existing repo, idempotently. See
        // `router/repository.rs::attach_remote`.
        .route(
            "/api/repository/{repo}/remote",
            post(repository::attach_remote),
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
        .route(
            "/api/repository/{repo}/branch/{branch}/sync/status",
            get(sync::sync_status),
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
        // Evaluate route — accepts an asserted-notation document
        // (any mix of queries and mutations), runs the unified
        // analyze → query → plan → commit pipeline, and returns
        // matches plus a commit summary in one response.
        .route(
            "/api/repository/{repo}/branch/{branch}/evaluate",
            post(evaluate::evaluate),
        )
        // CSV export / import — stream the branch's artifacts out as
        // `text/csv`, or commit a CSV body's rows as assertions. See
        // `router/transfer.rs`.
        .route(
            "/api/repository/{repo}/branch/{branch}/export",
            get(transfer::export),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/import",
            post(transfer::import),
        )
        // Structured-mutation route — see `plan/transact-endpoint.md`.
        // Bypasses tonk-notation: accepts a typed
        // `TransactRequest` so per-mutation transient/durable
        // classification flows straight to the reactor's
        // transaction builder.
        .route(
            "/api/repository/{repo}/branch/{branch}/transact",
            post(transact::transact),
        )
        // Query route — accepts a serialized `ConceptQuery`,
        // returns conclusions. With `Accept: text/event-stream`
        // the response is an SSE subscription that re-broadcasts
        // whenever the branch changes.
        .route(
            "/api/repository/{repo}/branch/{branch}/query",
            post(query::query),
        )
        // Host/guest iframe bridge. The shell embeds an iframe
        // pointed at this URL; the handler records the iframe's
        // client id against `{repo, branch}` so its later
        // subresource fetches can be re-rooted, and serves the
        // entity's body by selecting `(the=<mime>, of=<entity>)`
        // on the branch. The MIME comes from the entity's
        // trailing `.<ext>` (defaulting to `text/html`).
        .route(
            "/api/repository/{repo}/branch/{branch}/host/{host}/{entity}",
            get(host::guest),
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
    use tonk_schema::prelude::DidExt as _;

    use crate::worker::DefaultSpace;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Creates a test state with the default storage backend.
    ///
    /// The state has a profile and operator but *no* repository —
    /// tests that need one call [`put_repo`] with a display label and
    /// use the minted routing key it returns. Every create mints a
    /// fresh identity, so repeated runs never collide even when they
    /// share a label.
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

        let reactor = crate::TonkReactor::new(profile.clone());
        TonkState {
            profile,
            operator,
            profile_name: "test-tonk".to_string(),
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            commands: super::command_registry(),
        }
    }

    /// Creates a test repository via `PUT /api/repository/{label}` and
    /// returns its minted routing key.
    ///
    /// `label` is only a display name now — the repository's identity is
    /// a freshly minted `did:key`, and the routing key returned here (the
    /// DID suffix from the 201 `RepositoryInfo`) is what every subsequent
    /// request must address. Each PUT always creates, so runs are
    /// independent without name juggling.
    async fn put_repo(app: &Router, label: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", label))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "expected 201 from PUT /api/repository/{label}, got {status}",
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        info.name
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

        // The PUT path segment is only a display label; the response
        // identifier is the freshly minted routing key (the DID suffix),
        // which is what subsequent requests address.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/test-create")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        // `name` is the routing key (the DID suffix). `label` reads from
        // the repository's own `tonk/repository` name on its content
        // branch; this branchless PUT seeds no content branch (and thus
        // no name), so the label falls back to the routing key.
        assert_eq!(resp.name, resp.subject.repo_key());
        assert_eq!(resp.label, resp.name);
        assert!(!resp.subject.as_str().is_empty());

        // The returned key is addressable; GET it back and confirm the
        // routing key and the (key-fallback) label are stable.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}", resp.name))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(fetched.name, resp.name, "routing key is stable across GET");
        assert_eq!(
            fetched.label, resp.name,
            "label falls back to the routing key when no name is seeded",
        );
        assert_eq!(fetched.subject, resp.subject);
    }

    #[dialog_common::test]
    async fn it_mints_a_fresh_key_for_each_create_under_the_same_label() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        // Two PUTs to the same label both succeed (no collision) and
        // mint distinct routing keys — identity is the minted DID, the
        // label is only a display name.
        let first = put_repo(&app, "duplicate-label").await;
        let second = put_repo(&app, "duplicate-label").await;
        assert_ne!(
            first, second,
            "each create must mint a distinct routing key",
        );
    }

    #[dialog_common::test]
    async fn it_routes_invite_minting() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-invite-route";

        // Create the repo first so the invite handler can load it.
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

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
    async fn post_join(app: &Router, url: &str) -> (StatusCode, serde_json::Value) {
        let body = serde_json::json!({ "url": url }).to_string();
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
        let (status, body) = post_join(&app, &invite_url).await;

        assert_eq!(
            status,
            StatusCode::CREATED,
            "expected 201 Created on first join, got {status}: {body}",
        );
        assert_eq!(body["outcome"], "joined", "expected joined outcome: {body}");
        // The repository identifier is the subject's routing key (the DID
        // suffix). Join no longer takes a local name — the display name
        // comes from the shared content branch — so the label falls back
        // to the routing key until that branch syncs.
        assert_eq!(body["repository"]["name"], subject_did.repo_key());
        assert_eq!(body["repository"]["label"], subject_did.repo_key());
        assert_eq!(body["repository"]["subject"], subject_did.to_string());
    }

    #[dialog_common::test]
    async fn it_renews_when_subject_already_mounted() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let (invite_url, subject_did) = synthesize_open_invite().await;

        // First join creates the replica under the subject's routing key.
        let (first_status, first_body) = post_join(&app, &invite_url).await;
        assert_eq!(
            first_status,
            StatusCode::CREATED,
            "first join: {first_body}"
        );

        // Second join of the *same invite URL* — same subject, the
        // recipient already has it mounted. Worker should respond with a
        // `renewed` outcome and return the existing replica, keyed by the
        // subject's identity.
        let (second_status, second_body) = post_join(&app, &invite_url).await;
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
            second_body["repository"]["name"],
            subject_did.repo_key(),
            "renewed should return the existing replica keyed by its identity",
        );
    }

    #[dialog_common::test]
    async fn it_rejects_malformed_invite_url() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let (status, _body) = post_join(&app, "not-a-url").await;
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

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

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

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

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

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

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
    async fn it_sweeps_a_tagged_repo_with_no_upstream_branches_as_a_no_op() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());
        let repo = "test-bgsync-noupstream";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // No branch has an upstream, so the worker-side sweep selects
        // nothing and runs the `/sync` route zero times — a clean
        // resolve, not a rejection.
        super::sync_repository(&app_state, repo)
            .await
            .expect("a no-upstream repo should sweep cleanly");
    }

    #[dialog_common::test]
    async fn it_treats_a_tag_for_an_unknown_repo_as_a_no_op() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));

        // Nothing to retry for a repo that does not exist, so the
        // sweep resolves rather than rejecting.
        super::sync_repository(&app_state, "no-such-repo")
            .await
            .expect("an unknown repo should resolve as a no-op");
    }

    #[dialog_common::test]
    async fn it_reports_no_upstream_status_for_an_unconfigured_branch() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-sync-status";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Land a commit so the branch has a local revision — the
        // status route should still report `no-upstream` (none is
        // configured) while surfacing the local head.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/claim/assert/test:status/test/value",
                        repo
                    ))
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("status test"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/sync/status", repo))
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: super::SyncStatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.state, tonk_schema::SyncState::NoUpstream);
        assert!(
            status.local.is_some(),
            "the local head should be reported even with no upstream"
        );
        assert!(status.remote.is_none(), "no upstream means no remote head");
    }

    /// Wire `main`'s upstream to a sibling `upstream` branch in the
    /// same repo — the in-process stand-in for a real remote, the
    /// same pattern the slide sync tests use — so the status route's
    /// fetch + classify path has somewhere local to read from.
    /// Returns the router for driving the HTTP surface.
    async fn repo_with_sibling_upstream(label: &str) -> (Router, String) {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());
        let repo = put_repo(&app, label).await;
        let repo = repo.as_str();

        let guard = app_state.read().await;
        let repo_state = guard
            .reactor
            .repository(repo)
            .acquire(&guard.operator)
            .await
            .expect("acquire repository");
        let upstream = repo_state
            .repository()
            .branch("upstream")
            .open()
            .perform(&guard.operator)
            .await
            .expect("open sibling upstream branch");
        let main = guard
            .reactor
            .repository(repo)
            .branch("main")
            .acquire(&guard.operator)
            .await
            .expect("acquire main branch");
        main.handle()
            .set_upstream(&upstream)
            .perform(&guard.operator)
            .await
            .expect("set main's upstream to the sibling");
        drop(guard);
        (app, repo.to_owned())
    }

    /// Land a commit on `main` by asserting one marker fact. Each
    /// distinct `marker` is its own entity, so successive calls
    /// advance the tree.
    async fn commit_marker(app: &Router, repo: &str, marker: &str) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{repo}/branch/main/claim/assert/test:{marker}/test/value"
                    ))
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from(marker.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "commit '{marker}' should land"
        );
    }

    /// Push `main` to its upstream over the HTTP sync route, asserting
    /// the push reports success.
    async fn push_main(app: &Router, repo: &str) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/sync/push"))
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
        let sync: super::SyncResponse = serde_json::from_slice(&body).unwrap();
        assert!(sync.success, "push should succeed: {:?}", sync.error);
    }

    /// GET the sync status of `main` and deserialize the response.
    async fn get_main_status(app: &Router, repo: &str) -> super::SyncStatusResponse {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/sync/status"))
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[dialog_common::test]
    async fn it_reports_synced_status_after_pushing_to_the_upstream() {
        let (app, repo) = repo_with_sibling_upstream("test-sync-status-synced").await;

        commit_marker(&app, &repo, "synced-probe").await;
        push_main(&app, &repo).await;

        // Both heads now populated and equal — exercises the route's
        // fetch + classify path and the both-revisions-present JSON
        // shape, not just the no-upstream early return.
        let status = get_main_status(&app, &repo).await;
        assert_eq!(status.state, tonk_schema::SyncState::Synced);
        let local = status.local.expect("local head present");
        let remote = status.remote.expect("remote head present after push");
        assert_eq!(local.tree, remote.tree, "synced means the heads match");
    }

    #[dialog_common::test]
    async fn it_reports_ahead_status_when_local_leads_the_upstream() {
        let (app, repo) = repo_with_sibling_upstream("test-sync-status-ahead").await;

        // Establish a shared base on the upstream, then advance main
        // one commit past it.
        commit_marker(&app, &repo, "base").await;
        push_main(&app, &repo).await;
        commit_marker(&app, &repo, "ahead-probe").await;

        let status = get_main_status(&app, &repo).await;
        assert_eq!(status.state, tonk_schema::SyncState::Ahead);
        assert!(status.local.is_some(), "local head present");
        assert!(
            status.remote.is_some(),
            "remote head present — the shared base the upstream still points at"
        );
    }

    #[dialog_common::test]
    async fn it_inspects_branch_after_commit() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-inspect";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

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

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let body = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name


attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  description: A person
  with:
    name: person-name
    age:  person-age
";

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
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
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(resp.commits.claims > 0);
        // Three declared bookmarks → three entities surfaced.
        let person = resp.commits.entities.get("person").expect("person entity");
        assert!(person.starts_with("concept:"));
    }

    /// A single `attribute!` assertion commits cleanly via the
    /// new YAML notation.
    #[dialog_common::test]
    async fn it_transacts_a_single_attribute() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-single";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let body = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name
";

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
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
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        // The bookmark `person-name` becomes a content-derived
        // attribute entity surfaced under the bookmark's name.
        let entity = resp
            .commits
            .entities
            .get("person-name")
            .expect("person-name entity");
        assert!(entity.starts_with("the:"));
    }

    /// Sending a parser-rejected document returns 400 with a
    /// human-readable error.
    #[dialog_common::test]
    async fn it_returns_400_on_malformed_transaction() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-malformed";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // YAML with an indentation error — saphyr rejects it.
        let body = "person!:\n  name: Alice\n bad-indent: 1\n";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
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

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // First transaction: define `person-name` only.
        let first = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
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
attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  description: A person
  with:
    name: person-name
    age:  person-age
";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
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
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let entity = resp.commits.entities.get("person").unwrap();
        assert!(entity.starts_with("concept:"));
    }

    /// End-to-end concept retraction: define schema, assert
    /// an instance, then retract the concept-projection — the
    /// worker should query for the instance's current values
    /// and dissociate them.
    #[dialog_common::test]
    async fn it_retracts_a_concept_instance() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-retract-concept";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Define schema + assert an Alice with a bookmark
        // binding so we know her entity.
        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &alice
  name: \"Alice\"
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(setup))
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
            "setup failed: {}",
            String::from_utf8_lossy(&body_bytes)
        );

        // Pull Alice's entity URI from the setup response —
        // bookmark heads put `name → entity` in `commits.entities`.
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let alice_uri = setup_resp
            .commits
            .entities
            .get("alice")
            .expect("setup should bind `alice` to an entity")
            .clone();
        // Retract the person concept-projection from Alice by
        // that URI. The worker should query the branch for
        // `(io.gozala.person/name, alice, *)` and dissociate
        // the matching value.
        let retract = format!("person!:\n  this: {alice_uri}\n  ..: _\n");
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(retract))
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
            "retraction failed: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        // One claim retracted (Alice's name).
        assert!(
            resp.commits.claims >= 1,
            "expected at least 1 retracted claim, got {}",
            resp.commits.claims
        );
    }

    /// Anonymous-head query (`person:`) must not crash dialog
    /// with `UnboundVariable { variable_name: "this" }`. The
    /// analyzer mints a named variable for `this` so the engine
    /// can bind matches; the `/evaluate` route exercises that
    /// path end-to-end.
    #[dialog_common::test]
    async fn it_runs_anonymous_query() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-anon-query";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Define a `person` concept and assert one Alice so the
        // query has something to match.
        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &alice
  name: \"Alice\"
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(setup))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        // The actual regression: `person:` (no `?var`, no `!`).
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from("person:\n  name: \"Alice\"\n"))
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
            "anonymous query failed: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(resp.matches_after.len(), 1, "expected 1 match block");
        assert_eq!(
            resp.matches_after[0].results.len(),
            1,
            "expected 1 result for Alice"
        );
        // Empty-body anonymous query (`person:`) reads as
        // `person:\n  name: ?name\n  …` — every field of the
        // concept is surfaced in the result, not just `this`.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from("person:\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(resp.matches_after[0].results.len(), 1);
        let alice = &resp.matches_after[0].results[0];
        assert_eq!(
            alice.fields.get("name"),
            Some(&serde_json::json!("Alice")),
            "empty-body query should surface the `name` field; got {:?}",
            alice.fields
        );
    }

    /// Cardinality-one assert against an existing entity must
    /// *replace* the prior value, not accumulate alongside it.
    /// Dialog's storage layer is additive, so the worker has to
    /// query-then-dissociate the prior value before the new
    /// associate. Three update paths are exercised:
    /// query-bound `?var`, explicit URI, and re-asserted
    /// bookmark on the same body (no-op).
    #[dialog_common::test]
    async fn it_supersedes_cardinality_one_field_on_update() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-supersede";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Setup: schema + Alice with age 28.
        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  with:
    name: person-name
    age:  person-age

person!: &alice
  name: \"Alice\"
  age:  28
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(setup))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let alice_uri = setup_resp.commits.entities.get("alice").cloned().unwrap();

        // --- Path 1: query-bound `?alice` then assert.
        let update_via_query = "\
person:
  this: ?alice
  name: \"Alice\"
person!:
  this: ?alice
  age:  29
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(update_via_query))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        // Anonymous query must now see exactly one Alice with
        // age = 29 — not two ages, not the old 28.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from("person:\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            resp.matches_after[0].results.len(),
            1,
            "expected exactly 1 person after update; got {:?}",
            resp.matches_after[0].results
        );
        assert_eq!(
            resp.matches_after[0].results[0].fields.get("age"),
            Some(&serde_json::json!(29)),
            "age should be 29 after `?alice` update; got {:?}",
            resp.matches_after[0].results[0].fields
        );

        // --- Path 2: explicit URI assert.
        let update_via_uri = format!("person!:\n  this: {alice_uri}\n  age: 30\n");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(update_via_uri))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from("person:\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(resp.matches_after[0].results.len(), 1);
        assert_eq!(
            resp.matches_after[0].results[0].fields.get("age"),
            Some(&serde_json::json!(30)),
            "age should be 30 after URI update; got {:?}",
            resp.matches_after[0].results[0].fields
        );

        // --- Path 3: re-assert the same value should be a
        // no-op (no new claim, no churn).
        let reassert = format!("person!:\n  this: {alice_uri}\n  age: 30\n");
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(reassert))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Anchor git-tag rebind: `person!: &alice` with a different
    /// body derives a *new* entity from the new body and rebinds
    /// the `id:alice → entity` claim from the old entity to the
    /// new one (cardinality-one supersession on
    /// `dialog.meta/name`). After the rebind, the old entity
    /// still
    /// holds its facts but is no longer addressable by `.alice`.
    #[dialog_common::test]
    async fn it_rebinds_bookmark_on_body_change() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-rebind-bookmark";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  with:
    name: person-name
    age:  person-age

person!: &alice
  name: \"Alice\"
  age:  28
";
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let alice_v1 = setup_resp.commits.entities.get("alice").cloned().unwrap();

        // Same anchor name, different body → new entity.
        let rebind = "\
person!: &alice
  name: \"Alice\"
  age:  29
";
        let body_bytes = post_yaml(&app, repo, rebind).await;
        let rebind_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let alice_v2 = rebind_resp.commits.entities.get("alice").cloned().unwrap();
        assert_ne!(
            alice_v1, alice_v2,
            "different body must produce a different bookmark target"
        );

        // .person-name reference in field position resolves
        // through the same in-doc declarations / branch-side
        // attribute lookup. Test cross-doc bookmark resolution
        // by referencing `.person-name` from a follow-up doc.
        let follow_up = "\
person:
  this: ?p
  name: ?n
";
        let body_bytes = post_yaml(&app, repo, follow_up).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        // Both entities should still match the `person` concept
        // (we never retracted v1's facts) — but only v2 should
        // be addressable via `.alice`.
        assert!(
            resp.matches_after[0].results.len() >= 1,
            "expected at least one match"
        );
    }

    /// Anonymous head (`person!:`) mints a body-derived entity,
    /// no name claim. Re-running the same body is a no-op (same
    /// hash → same entity → cardinality-one supersession).
    #[dialog_common::test]
    async fn it_mints_anonymous_head_from_body() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-anon-head";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name
";
        let _ = post_yaml(&app, repo, setup).await;

        // First commit: anonymous head, body-derived entity.
        let body = "person!:\n  name: \"Bob\"\n";
        let body_bytes = post_yaml(&app, repo, body).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(resp.commits.claims >= 1);

        // Query — exactly one Bob.
        let body_bytes = post_yaml(&app, repo, "person:\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let bobs: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.fields.get("name") == Some(&serde_json::json!("Bob")))
            .collect();
        assert_eq!(bobs.len(), 1, "expected exactly 1 Bob; got {bobs:?}");

        // Re-commit same body — still exactly one Bob.
        let _ = post_yaml(&app, repo, body).await;
        let body_bytes = post_yaml(&app, repo, "person:\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let bobs: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.fields.get("name") == Some(&serde_json::json!("Bob")))
            .collect();
        assert_eq!(
            bobs.len(),
            1,
            "anonymous head re-asserted same body should be no-op; got {bobs:?}"
        );
    }

    /// Variable-binding head with no preceding query mints an
    /// entity and registers `?var` for subsequent expressions in
    /// the same document.
    #[dialog_common::test]
    async fn it_introduces_variable_head_in_document_scope() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-var-intro";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  with:
    name: person-name
    age:  person-age
";
        let _ = post_yaml(&app, repo, setup).await;

        // Single head introduces `?carol` and writes both fields.
        // (Two heads with the same `person!:` body containing
        // `this: ?carol` would collapse to one expression at the
        // YAML mapping-key level — that's a parser-side detail,
        // not a semantic limit of variable scope.)
        let doc = "\
person!:
  this: ?carol
  name: \"Carol\"
  age:  31
";
        let body_bytes = post_yaml(&app, repo, doc).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let carol_uri = resp
            .commits
            .entities
            .get("?carol")
            .cloned()
            .expect("?carol should be surfaced in commits.entities");

        // Verify both facts landed on the same entity.
        let body_bytes = post_yaml(&app, repo, "person:\n  name: \"Carol\"\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let carols: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.this == carol_uri)
            .collect();
        assert_eq!(carols.len(), 1, "expected one Carol at {carol_uri}");
        assert_eq!(carols[0].fields.get("age"), Some(&serde_json::json!(31)));
    }

    /// Cardinality-many fields stay additive — re-asserting
    /// adds another value rather than superseding.
    #[dialog_common::test]
    async fn it_accumulates_cardinality_many_field() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-many";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &tagged-name
  the:         io.gozala.tagged/name
  as:          text
  cardinality: one
  description: Name of the tagged item

attribute!: &tagged-tag
  the:         io.gozala.tagged/tag
  as:          text
  cardinality: many
  description: Tags applied to the item

concept!: &tagged
  with:
    name: tagged-name
    tag:  tagged-tag

tagged!: &dave
  name: \"Dave\"
  tag:  \"engineer\"
";
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let dave_uri = setup_resp.commits.entities.get("dave").cloned().unwrap();

        // Add a second tag using URI binding (avoids body
        // hashing producing a new entity).
        let add_tag = format!("tagged!:\n  this: {dave_uri}\n  tag: \"author\"\n");
        let _ = post_yaml(&app, repo, &add_tag).await;

        // Both tags should be on Dave. The query renders one
        // tag value at a time (cardinality-many surfaces as
        // multiple result rows in dialog), so look at the raw
        // claim count via a tag-only query.
        let body_bytes = post_yaml(&app, repo, "tagged:\n  tag: ?tag\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let dave_tags: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.this == dave_uri)
            .filter_map(|r| r.fields.get("tag"))
            .collect();
        assert_eq!(
            dave_tags.len(),
            2,
            "cardinality-many should accumulate; got {dave_tags:?}"
        );
    }

    /// Concept-level retraction by URI removes every fact in
    /// the concept's `with` map for the entity.
    #[dialog_common::test]
    async fn it_retracts_concept_projection_by_uri() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-retract-uri";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  with:
    name: person-name
    age:  person-age

person!: &erin
  name: \"Erin\"
  age:  41
";
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let erin_uri = setup_resp.commits.entities.get("erin").cloned().unwrap();

        // Retract the projection.
        let retract = format!("person!:\n  this: {erin_uri}\n  ..: _\n");
        let body_bytes = post_yaml(&app, repo, &retract).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            resp.commits.claims >= 2,
            "expected at least 2 retracted claims (name + age); got {}",
            resp.commits.claims
        );

        // Querying for Erin should now return no matches.
        let body_bytes = post_yaml(&app, repo, "person:\n  name: \"Erin\"\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let erins: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.fields.get("name") == Some(&serde_json::json!("Erin")))
            .collect();
        assert!(
            erins.is_empty(),
            "Erin should be gone after retraction; got {erins:?}"
        );
    }

    /// Concept-level retraction via query-bound `?var` removes
    /// the projection for every match.
    #[dialog_common::test]
    async fn it_retracts_concept_projection_via_query() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-retract-var";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &frank
  name: \"Frank\"
";
        let _ = post_yaml(&app, repo, setup).await;

        let retract = "\
person:
  this: ?frank
  name: \"Frank\"
person!:
  this: ?frank
  ..: _
";
        let body_bytes = post_yaml(&app, repo, retract).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            resp.commits.claims >= 1,
            "expected ≥1 retracted claim; got {}",
            resp.commits.claims
        );

        // Frank gone.
        let body_bytes = post_yaml(&app, repo, "person:\n  name: \"Frank\"\n").await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let franks: Vec<_> = resp.matches_after[0]
            .results
            .iter()
            .filter(|r| r.fields.get("name") == Some(&serde_json::json!("Frank")))
            .collect();
        assert!(franks.is_empty(), "Frank should be gone; got {franks:?}");
    }

    /// Multi-expression query joins on shared variables. Two
    /// query expressions sharing `?p` filter to entities that
    /// match both.
    #[dialog_common::test]
    async fn it_joins_queries_on_shared_variable() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-join";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Two concepts that overlap on a `name` field via
        // different attribute namespaces. We'll query for
        // entities present in both.
        let setup = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

attribute!: &employee-id
  the:         io.gozala.employee/id
  as:          text
  cardinality: one
  description: Employee identifier

concept!: &person
  with:
    name: person-name

concept!: &employee
  with:
    eid: employee-id

person!: &gina
  name: \"Gina\"
";
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let gina_uri = setup_resp.commits.entities.get("gina").cloned().unwrap();

        // Add an employee fact on Gina's entity.
        let add_emp = format!("employee!:\n  this: {gina_uri}\n  eid: \"E-007\"\n");
        let _ = post_yaml(&app, repo, &add_emp).await;

        // Joined query.
        let join = "\
person:
  this: ?p
  name: ?n
employee:
  this: ?p
  eid: ?e
";
        let body_bytes = post_yaml(&app, repo, join).await;
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(resp.matches_after.len(), 2, "expected 2 query blocks");
        // Both blocks should surface Gina's entity (the join
        // filters to the shared row).
        for block in &resp.matches_after {
            let on_gina: Vec<_> = block
                .results
                .iter()
                .filter(|r| r.this == gina_uri)
                .collect();
            assert_eq!(
                on_gina.len(),
                1,
                "block {} should match Gina; got {:?}",
                block.label,
                block.results
            );
        }
    }

    /// Helper: POST a YAML body to /evaluate and return raw
    /// response bytes (asserting a 200). Cuts the per-call
    /// boilerplate that was making the matrix tests above
    /// dense to read.
    async fn post_yaml(app: &Router, repo: &str, body: &str) -> Vec<u8> {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(body.to_owned()))
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
            "evaluate failed: {}",
            String::from_utf8_lossy(&body_bytes)
        );
        body_bytes.to_vec()
    }

    // ---------------------------------------------------------- //
    // Reactor: /api/repository/{repo}/branch/{branch}/query      //
    // ---------------------------------------------------------- //

    use crate::helpers::named_concept_wire_query;

    /// Seed one named entity on `main` so the reactor tests have
    /// something to query.
    async fn seed_named_entity(app: &Router, repo: &str) {
        seed_named_attribute(app, repo, "person-name", "xyz.tonk.person/name").await;
    }

    /// Seed an attribute definition with a chosen name and `the:`
    /// IRI so tests can grow the result set across commits.
    async fn seed_named_attribute(app: &Router, repo: &str, name: &str, the: &str) {
        let body = format!(
            "\
attribute!: &{name}
  the:         {the}
  as:          text
  cardinality: one
  description: A test attribute
"
        );
        let _ = post_yaml(app, repo, &body).await;
    }

    /// Run a one-shot `/query` request and return the parsed
    /// JSON body.
    async fn post_query(app: &Router, repo: &str, branch: &str) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/{branch}/query"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(named_concept_wire_query().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).expect("query response is JSON")
    }

    /// Run a one-shot `/query` with a formula predicate (a bare
    /// string), returning the parsed JSON body and HTTP status.
    async fn post_formula_query(
        app: &Router,
        repo: &str,
        branch: &str,
        formula: &str,
    ) -> (StatusCode, serde_json::Value) {
        let wire = serde_json::json!({ "predicate": formula, "terms": {} });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/{branch}/query"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(wire.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// A `tree/node` formula query walks the branch's index tree:
    /// it reads and decodes the root node, returning a `Conclusion`
    /// whose `this` is the node hash and whose fields describe the
    /// node (kind, byte size, child/entry count).
    #[dialog_common::test]
    async fn it_resolves_a_tree_node_formula() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-formula";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        let (status, body) = post_formula_query(&app, repo, "main", "tree/node").await;
        assert_eq!(status, StatusCode::OK, "formula query OK: {body}");
        let arr = body.as_array().expect("array of conclusions");
        assert_eq!(arr.len(), 1, "a seeded branch has a root node: {body}");
        let row = &arr[0];
        assert!(
            row["this"].as_str().is_some_and(|s| s.starts_with('#')),
            "this is a base58 node hash: {row}"
        );
        let kind = row["fields"]["kind"].as_str().unwrap_or("");
        assert!(kind == "branch" || kind == "leaf", "kind set: {row}");
        assert!(
            row["fields"]["size"].as_i64().is_some_and(|n| n > 0),
            "node has a byte size: {row}"
        );
        assert!(
            row["fields"]["count"].as_i64().is_some(),
            "node has a child/entry count: {row}"
        );
    }

    /// An unknown formula name is a 4xx, not a panic or a concept
    /// fall-through.
    #[dialog_common::test]
    async fn it_rejects_an_unknown_formula() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-formula-unknown";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        let (status, _body) = post_formula_query(&app, repo, "main", "tree/bogus").await;
        assert_ne!(status, StatusCode::OK, "unknown formula must not be OK");
    }

    /// Open an SSE subscription and return the open body so the
    /// caller can read frames as they arrive.
    async fn open_subscription(app: &Router, repo: &str, branch: &str) -> Body {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/{branch}/query"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .header("accept", "text/event-stream")
                    .body(Body::from(named_concept_wire_query().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap_or("")),
            Some("text/event-stream"),
        );
        response.into_body()
    }

    /// Read one SSE `data: <json>\n\n` frame off `body` and parse
    /// the JSON payload.
    async fn read_sse_frame(body: &mut Body) -> serde_json::Value {
        use http_body_util::BodyExt as _;
        let frame = body
            .frame()
            .await
            .expect("at least one frame")
            .expect("frame ok");
        let bytes = frame.into_data().expect("data frame");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        let json_text = text
            .strip_prefix("data: ")
            .and_then(|s| s.strip_suffix("\n\n"))
            .expect("SSE-framed body");
        serde_json::from_str(json_text).expect("snapshot is JSON")
    }

    /// Open an SSE subscription, read the first event, and return
    /// the parsed snapshot. Drops the body afterwards.
    async fn subscribe_first_event(app: &Router, repo: &str, branch: &str) -> serde_json::Value {
        let mut body = open_subscription(app, repo, branch).await;
        read_sse_frame(&mut body).await
    }

    /// One-shot `/query` returns the current matches as a JSON
    /// array of `Conclusion`. Each row carries a `this`
    /// entity URI.
    #[dialog_common::test]
    async fn it_returns_query_results_one_shot() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-query";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        let body = post_query(&app, repo, "main").await;
        let arr = body.as_array().expect("array");
        assert!(
            !arr.is_empty(),
            "expected at least one named entity after seeding, got {body}"
        );
        for row in arr {
            assert!(row.get("this").is_some(), "every row carries `this`: {row}");
        }
    }

    /// SSE `/query` opens a stream whose first event is the
    /// current snapshot — the same payload the one-shot route
    /// returns inline.
    #[dialog_common::test]
    async fn it_streams_initial_snapshot_over_sse() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-sse";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        let snapshot = subscribe_first_event(&app, repo, "main").await;
        let arr = snapshot.as_array().expect("array");
        assert!(!arr.is_empty(), "snapshot non-empty");

        let oneshot = post_query(&app, repo, "main").await;
        assert_eq!(snapshot, oneshot, "SSE snapshot matches one-shot result");
    }

    /// Subscribing twice with the same query against the same
    /// branch must collide on the subscription's content hash —
    /// both subscribers attach to the same `Subscription` and
    /// each sees a snapshot.
    #[dialog_common::test]
    async fn it_shares_subscription_across_subscribers_with_same_query() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-shared";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        let first = subscribe_first_event(&app, repo, "main").await;
        let second = subscribe_first_event(&app, repo, "main").await;

        assert_eq!(
            first, second,
            "both subscribers receive the same snapshot bytes"
        );
    }

    /// Malformed request body returns 400 with a parse error
    /// rather than crashing.
    #[dialog_common::test]
    async fn it_rejects_malformed_query_body() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-malformed";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/query"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{ this is not valid JSON"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// A commit on a branch must fan out a fresh SSE event to
    /// every subscriber whose query result changed. This is the
    /// load-bearing path of the whole reactor.
    #[dialog_common::test]
    async fn it_broadcasts_changed_snapshot_after_commit() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-broadcast";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        let mut body = open_subscription(&app, repo, "main").await;
        let snapshot_before = read_sse_frame(&mut body).await;

        // Commit a second named attribute — the result set grows.
        seed_named_attribute(&app, repo, "person-age", "xyz.tonk.person/age").await;

        let snapshot_after = read_sse_frame(&mut body).await;
        assert_ne!(
            snapshot_before, snapshot_after,
            "commit must broadcast a changed snapshot"
        );
        assert!(
            snapshot_after.as_array().map(|a| a.len()).unwrap_or(0)
                > snapshot_before.as_array().map(|a| a.len()).unwrap_or(0),
            "post-commit snapshot has more rows than pre-commit"
        );
    }

    /// Re-polling a subscription whose result didn't change must
    /// not push another SSE event. The poll path hashes the new
    /// payload and compares against `last_hash`; equal hashes
    /// short-circuit.
    #[dialog_common::test]
    async fn it_skips_broadcast_when_repoll_is_unchanged() {
        use http_body_util::BodyExt as _;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());

        let repo = "test-reactor-noop-repoll";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        let mut body = open_subscription(&app, repo, "main").await;
        let _snapshot = read_sse_frame(&mut body).await;

        // Trigger a re-poll directly via the reactor — same query,
        // same data, same hash, so the dedup path must skip the
        // broadcast and the subscriber must not receive a frame.
        {
            let guard = app_state.read().await;
            let session = guard
                .reactor
                .repository(repo)
                .branch("main")
                .acquire(&guard.operator)
                .await
                .expect("acquire");
            session.state.poll(&guard.operator).await;
        }

        // Race the next frame against a short sleep. We expect
        // the sleep to win — no frame should arrive.
        let next_frame = body.frame();
        let timeout = crate::sleep(web_time::Duration::from_millis(200));
        tokio::select! {
            frame = next_frame => panic!("unexpected SSE frame after no-op repoll: {frame:?}"),
            _ = timeout => {} // expected — no broadcast
        }
    }

    /// When a subscriber's receiver closes (the SSE body is
    /// dropped), the next change-driven re-poll must evict that
    /// subscriber. With no subscribers left, the subscription
    /// itself is removed from the branch's map. Pruning is
    /// piggy-backed on the send attempt — there is no separate
    /// reaper — so this test drives a commit to trigger the send.
    #[dialog_common::test]
    async fn it_prunes_dropped_subscriber_on_change() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());

        let repo = "test-reactor-prune";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        // Subscribe and read the first frame so the subscriber is
        // registered with `Status::Established`.
        {
            let mut body = open_subscription(&app, repo, "main").await;
            let _snapshot = read_sse_frame(&mut body).await;
            // body drops here, closing the receiver.
        }

        let session = {
            let guard = app_state.read().await;
            guard
                .reactor
                .repository(repo)
                .branch("main")
                .acquire(&guard.operator)
                .await
                .expect("acquire")
        };
        assert_eq!(
            session.state.subscriptions().lock().len(),
            1,
            "subscription registered before prune"
        );

        // Commit a change so the next poll has new bytes to
        // broadcast. The send to the dead channel fails,
        // `retain_mut` drops the subscriber, and with the list
        // empty the subscription itself is removed.
        seed_named_attribute(&app, repo, "person-age", "xyz.tonk.person/age").await;

        assert!(
            session.state.subscriptions().lock().is_empty(),
            "dropped subscriber's subscription must be pruned after a change-driven poll"
        );
    }

    /// One-shot `/query` projects every term named in the
    /// query's `terms` map into [`Conclusion::fields`]. The
    /// `Name` view binds `this` (the `id:<n>` name entity) and
    /// `entity` (its current target), so each row must carry
    /// both.
    #[dialog_common::test]
    async fn it_projects_query_terms_into_conclusion_fields() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-projection";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        let body = post_query(&app, repo, "main").await;
        let arr = body.as_array().expect("array");
        assert!(!arr.is_empty(), "expected matches: {body}");

        for row in arr {
            let fields = row.get("fields").and_then(|f| f.as_object());
            let fields = fields.unwrap_or_else(|| panic!("row missing fields object: {row}"));
            let this = fields
                .get("this")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("fields[\"this\"] must be a string: {row}"));
            assert!(
                this.starts_with("id:"),
                "name entity `this` must be an `id:<n>` URI: {row}",
            );
            let entity = fields
                .get("entity")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("fields[\"entity\"] must be a string: {row}"));
            assert!(!entity.is_empty(), "entity target must be non-empty: {row}");
        }
    }

    /// Pull through the reactor's chain must re-poll
    /// subscriptions on success. We can't test against a real
    /// upstream here, but the no-op pull (no remote configured)
    /// still drives the chain and proves the wiring exists.
    /// After the pull, a subscription remains live and a
    /// follow-up commit broadcasts a fresh frame — proving the
    /// pull path didn't tear anything down.
    #[dialog_common::test]
    async fn it_keeps_subscription_live_across_pull() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());

        let repo = "test-reactor-pull";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        let mut body = open_subscription(&app, repo, "main").await;
        let snapshot_before = read_sse_frame(&mut body).await;

        // Pull through the reactor. With no upstream the dialog
        // pull is a no-op, but the reactor wiring still runs the
        // post-pull re-poll.
        let pull_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/sync/pull"))
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pull_response.status(), StatusCode::OK);

        // Subscription must still be alive — proven by a commit
        // delivering a fresh frame on the same SSE body.
        seed_named_attribute(&app, repo, "person-age", "xyz.tonk.person/age").await;
        let snapshot_after = read_sse_frame(&mut body).await;
        assert_ne!(
            snapshot_before, snapshot_after,
            "post-pull commit must still broadcast"
        );
    }

    /// `/query` against a missing repository returns 404, not
    /// 500.
    #[dialog_common::test]
    async fn it_returns_404_for_query_against_unknown_repo() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/no-such-repo/branch/main/query")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(named_concept_wire_query().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// `TonkReactor::shutdown` must drop every active subscriber's
    /// `mpsc::Sender` so the SSE response body finishes and the SW
    /// can release in-flight fetches.
    ///
    /// Drives the SW upgrade scenario without an actual SW: open a
    /// subscription, read its initial frame, call shutdown, then
    /// expect the body to end (next frame yields `None`).
    #[dialog_common::test]
    async fn it_releases_sse_subscribers_on_shutdown() {
        use http_body_util::BodyExt as _;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());

        let repo = "test-reactor-shutdown-releases";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_attribute(&app, repo, "person-name", "xyz.tonk.person/name").await;

        let mut body = open_subscription(&app, repo, "main").await;
        let _snapshot = read_sse_frame(&mut body).await;

        // Trigger the shutdown the SW upgrade path runs.
        {
            let guard = app_state.read().await;
            guard.reactor.shutdown();
        }

        // The body must end — `frame()` returns `None`. Race
        // against a timeout so a regression hangs the test rather
        // than passing on a delayed frame.
        let next_frame = body.frame();
        let timeout = crate::sleep(web_time::Duration::from_millis(500));
        tokio::select! {
            frame = next_frame => assert!(
                frame.is_none(),
                "expected stream end, got frame: {frame:?}",
            ),
            _ = timeout => panic!("subscription body did not end after shutdown"),
        }
    }

    // ---------------------------------------------------------- //
    // Regression tests pinning behaviours we've broken once
    // already and don't intend to break again.
    // ---------------------------------------------------------- //

    /// `transact=false` must not commit. Earlier the query
    /// parameter was parsed but ignored, so auto-evaluate from
    /// the editor was secretly applying every keystroke.
    /// Verify by counting commits before vs after a
    /// `transact=false` request.
    #[dialog_common::test]
    async fn it_does_not_commit_when_transact_false() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-no-commit-transact-false";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let body = "\
attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: \"name\"
";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/evaluate?transact=false",
                        repo
                    ))
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
            "expected 200; got {status}: {}",
            String::from_utf8_lossy(&body_bytes),
        );
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            resp.commits.claims,
            0,
            "transact=false must not commit any claims; got {} (response: {})",
            resp.commits.claims,
            String::from_utf8_lossy(&body_bytes),
        );
        assert_eq!(
            resp.revision_before, resp.revision_after,
            "transact=false must leave the branch revision unchanged",
        );
    }

    /// Worker rejections from the parser/analyzer must surface
    /// as a structured `kind: "analyze"` body with a `code` and
    /// `range` so the editor can position a squiggle. Previously
    /// they flattened to `kind: "router"` with neither, leaving
    /// the editor to silently drop the diagnostic.
    #[dialog_common::test]
    async fn it_returns_structured_analyze_error_for_malformed_body() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-analyze-error-shape";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // `name: x` is a head with a non-mapping body — the
        // parser rejects it.
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/evaluate?transact=false",
                        repo
                    ))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from("name: x"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            body["error"]["kind"], "analyze",
            "expected kind=analyze for editor squiggle routing; got {body}",
        );
        assert!(
            body["error"]["code"].is_string(),
            "expected a string `code` field for stable diagnostic routing; got {body}",
        );
        assert!(
            body["error"]["range"].is_object(),
            "expected a `range` object so the editor can position the squiggle; got {body}",
        );
    }

    /// `person!:` (an assertion with no explicit query) should
    /// produce a result block labelled `person`, not `?`. The
    /// implicit-query synthesizer mints a snapshot query for
    /// the touched entity; previously the renderer fell back to
    /// `?` because it only collected labels from explicit query
    /// expressions. The label now flows through
    /// `QueryAnalysis::labels`, populated by the analyzer for
    /// both explicit and implicit queries.
    #[dialog_common::test]
    async fn it_labels_implicit_query_block_with_assertion_head_name() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-implicit-query-label";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // First seed `person-name` and `person-age` attributes
        // so `person:` resolves; then assert `person!: …` on
        // its own (no explicit query expression) and check the
        // matches block label.
        let seed = "\
attribute!: &person-name
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
  description: \"name\"

attribute!: &person-age
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one
  description: \"age\"

concept!: &person
  description: \"a person\"
  with:
    name: person-name
    age:  person-age
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(seed))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let assertion = "\
person!:
  name: \"Bob\"
  age: 2
";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(assertion))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let labels: Vec<&str> = resp
            .matches_after
            .iter()
            .map(|b| b.label.as_str())
            .collect();
        assert!(
            labels.iter().any(|l| *l == "person"),
            "expected a result block labelled `person` for the implicit query; got {labels:?}",
        );
        assert!(
            !labels.iter().any(|l| *l == "?"),
            "result block must not fall back to `?` for an assertion with a known head; got {labels:?}",
        );
    }

    /// `field: _` in a query body means "match any value, don't
    /// bind it as a join key" — but the renderer should still
    /// project the matched value so the user sees it. The
    /// analyzer mints an auto-named variable for `_`; the
    /// renderer projects that under the user-facing field name.
    #[dialog_common::test]
    async fn it_renders_blank_query_field_with_matched_value() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-blank-field-render";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Seed person concept + an instance.
        let seed = "\
attribute!: &person-name
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
  description: \"name\"

attribute!: &person-age
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one
  description: \"age\"

concept!: &person
  description: \"a person\"
  with:
    name: person-name
    age:  person-age

person!:
  name: \"Alice\"
  age:  29
";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(seed))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Query with `name: _` — blank — and `age: _`. Expect
        // both fields to appear in the result with their
        // matched values.
        let query = "\
person:
  name: _
  age:  _
";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/evaluate", repo))
                    .method("POST")
                    .header("content-type", "application/yaml")
                    .body(Body::from(query))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let block = resp
            .matches_after
            .iter()
            .find(|b| b.label == "person")
            .expect("person result block");
        assert!(
            !block.results.is_empty(),
            "expected at least one match for the seeded person",
        );
        let row = &block.results[0];
        assert!(
            row.fields.contains_key("name"),
            "blank `name: _` field must still surface the matched value; got {:?}",
            row.fields,
        );
        assert!(
            row.fields.contains_key("age"),
            "blank `age: _` field must still surface the matched value; got {:?}",
            row.fields,
        );
    }

    /// An empty `TransactRequest` short-circuits without
    /// committing. Revisions match, claim count is zero, status
    /// is 200. Smoke test for route wiring and the no-op path.
    #[dialog_common::test]
    async fn it_transacts_empty_batch_as_noop() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-empty";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let body = "{\"claims\":[]}";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/json")
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
        assert_eq!(resp.commits.claims, 0);
        assert_eq!(resp.revision_before, resp.revision_after);
    }

    /// Malformed JSON body returns 400 with a router-level error.
    #[dialog_common::test]
    async fn it_returns_400_on_malformed_transact_body() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-bad-body";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let body = "not json at all";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/branch/main/transact", repo))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
