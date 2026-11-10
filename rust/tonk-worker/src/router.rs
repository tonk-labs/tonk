//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{
    Router,
    extract::State,
    middleware::{self, Next},
    response::Response,
    routing::get,
    routing::post,
    routing::put,
};

use crate::worker::TonkState;

/// Whether a newer service worker is installed and WAITING to take over —
/// i.e. whether this worker is retiring.
///
/// This is the browser-side race check. The Rust worker also owns a one-way
/// retirement latch set only after the successor installs successfully and
/// stream release begins. Routes consult both: the live slot closes the gap
/// before the latch is published, while the latch keeps new streams closed
/// after automatic activation clears `registration.waiting`.
///
/// Every route that opens a LONG-LIVED response must consult this. An
/// SSE body is a fetch event that never settles, and the spec keeps a
/// worker alive while any of its fetch events are in flight. A stream opened
/// during the handoff can therefore keep the retired incumbent serving stale
/// clients after the verified successor has activated.
///
/// `false` off-wasm and whenever the registration is unreadable — a
/// wrongly refused stream would starve consumers, a wrongly opened one
/// only delays an update.
pub(crate) fn update_pending() -> bool {
    false
}

mod claim;
pub use claim::{AssertPath, AssertResponse, ClaimQuery, ClaimResponse, QueryResponse};

pub(crate) mod account;
#[cfg(feature = "persona")]
mod account_deletion;
#[cfg(feature = "persona")]
mod account_journey;
#[cfg(feature = "persona")]
mod ceremony;
pub(crate) mod customer;
/// Account-bound email delivery for invitations.
#[cfg(feature = "persona")]
mod email_invitation;
#[cfg(all(
    test,
    feature = "integration-tests",
    feature = "persona",
    not(target_arch = "wasm32")
))]
mod identity_discovery_spike;
#[cfg(feature = "persona")]
mod link_timing;
/// Signing in, as this profile asks the host for it.
#[cfg(feature = "persona")]
pub(crate) mod registration;
#[cfg(feature = "persona")]
mod sign_in_via;

pub(crate) mod account_state;
pub use account_state::AccountKeys;

mod http;

pub(crate) mod adopt;
/// Getting the account's encryption key onto a device that needs it.
#[cfg(feature = "persona")]
pub(crate) mod custody;
/// Accreditation: rotate the onboarding account's custody to the passkey
/// account, then retire it.
pub(crate) mod rotation;

#[cfg(all(feature = "native-host", not(target_arch = "wasm32")))]
mod agent_link;
/// Acting as the account on a kernel, once the person lets it.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
mod authorize;
// Built natively too, without the feature, for this crate's own tests:
// the space writer is tested against a native access service.
#[cfg(any(feature = "volatile-storage", test))]
#[cfg_attr(not(feature = "volatile-storage"), allow(dead_code, unused_imports))]
mod hosted;
pub(crate) mod join;
/// Signing in on a kernel: the provisional persona merges into the
/// permanent one.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
mod merge;
/// A framed view's peer taking the delegation an MCP host signed to it.
#[cfg(feature = "persona")]
mod view_access;
#[cfg(all(feature = "native-host", not(target_arch = "wasm32")))]
pub use agent_link::AgentLinkWorker;
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub use authorize::{authorized, delegate_asked, unauthorized};
#[cfg(feature = "volatile-storage")]
pub use hosted::{
    HostedPeer, NODE_CACHE_BUDGET, SpaceWriter, WriterKey, WriterSession, join as agent_join, mcp,
    telemetry, view, writer_seed,
};
#[cfg(all(feature = "native-host", not(target_arch = "wasm32")))]
mod local_peer;
#[cfg(all(feature = "native-host", not(target_arch = "wasm32")))]
pub use local_peer::{LocalPeer, try_lock as try_lock_peer};
pub(crate) mod public_access;
mod publication;
/// Reaching a space's own worker from the person's profile.
pub(crate) mod space_directory;
/// Letting an agent or an email address into a space from the palette.
#[cfg(feature = "persona")]
mod space_invite;
pub(crate) mod space_reach;
/// A space's own worker: the delegation it holds, and the space it mounts.
#[cfg(target_arch = "wasm32")]
pub(crate) mod space_worker;
pub use join::{JoinRequest, JoinResponse};
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub use merge::{failed, merged, refused};
/// Spaces an earlier release kept in the app's origin, made again as
/// copies, on a kernel.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub mod earlier_release;
#[cfg(feature = "persona")]
mod local_space_link;
/// What a space's own worker asks of the persona, on a kernel.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub mod profile_asks;

#[cfg(feature = "persona")]
pub(crate) mod account_devices;

mod create_invite;
pub use create_invite::{CreateInviteRequest, CreateInviteResponse};

/// What an agent space reaches, as its kernel answers the hosted agent.
pub(crate) mod agent_access;
#[cfg(any(feature = "persona", feature = "connection-invites"))]
pub(crate) mod agent_connections;
#[cfg(feature = "persona")]
mod agent_session;
mod agent_space;
#[cfg(feature = "persona")]
mod agents;
pub(crate) mod revoke_invite;

/// Space membership management: admins and removals, as commands.
mod members;

/// Making a branch a space does not have yet, as a command.
mod branch;

pub mod inspect;
pub use inspect::{BranchStatusResponse, RemoteBranchStatusResponse, RemoteStatusResponse};

pub(crate) mod repository;
pub(crate) use repository::ProfileLibraryCache;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, bootstrap_profile,
};

/// The remotes a repository names, as its meta branch records them.
pub(crate) mod remotes;

mod sync;
pub use dialog_repository::Revision;
pub(crate) use sync::ProfileSwaps;
pub use sync::{
    SyncQueue, SyncResponse, SyncStatusResponse, branches_to_sync, drain_sync, sync_repository,
};
// Re-exported so API consumers (the UI) can name the state without
// depending on `tonk-schema` directly.
pub use tonk_schema::SyncState;

mod identify;
pub use identify::IdentifyResponse;

pub(crate) mod identity;

pub mod lsp;
pub use lsp::LspHub;

mod lsp_env;

pub(crate) mod profile;
pub use profile::{ProfileInfo, SpaceEntry};

#[cfg(feature = "persona")]
pub(crate) mod profiles;

#[cfg(feature = "persona")]
mod activation;
mod profile_name;

mod evaluate;
mod library;
mod seed;
pub use evaluate::{CommitSummary, EvaluatePath, EvaluateResponse, QueryMatchBlock, QueryResult};

mod query;
pub use query::QueryPath;

// Level 0 routing lives in `tonk-schema` (shared with the UI); re-export it so
// the SW's routing/containment code reads it locally.
pub use tonk_schema::{DEFAULT_BRANCH, SpaceRef, parse_space};

mod http_route;
mod session;
pub use session::{ClientRegistry, ClientState, Saved, SiteResponse, Stamp};

mod transact;
pub use transact::{ProfileTransactPath, TransactPath, TransactResponse};

/// The views a space renders, outlined for an agent.
#[cfg(any(target_arch = "wasm32", test))]
mod view_tree;

mod transfer;
pub use transfer::ImportResponse;

mod host;
pub use host::{ClientId, ViewBinding, ViewBindings};

mod blob;
pub(crate) mod renderer;

#[cfg(feature = "persona")]
mod migration;

mod interpret;
mod navigate;
#[cfg(feature = "persona")]
mod site_request;
mod site_select;

mod diagnostics;

mod command;
pub use command::{CommandEnv, CommandOrigin, CommandProviders, command_providers, dispatch};

#[cfg(test)]
mod intent;
#[cfg(test)]
mod route_table;
#[cfg(test)]
mod wire_compat;

/// Shared application state containing profile and operator.
pub type AppState = Arc<crate::StateLock<TonkState>>;

fn is_profile_scoped_path(path: &str) -> bool {
    path.starts_with("/api/")
        && !matches!(
            path,
            "/api/identify" | "/api/health" | "/api/inspect/diagnostics"
        )
}

async fn profile_context_fence(
    State(state): State<AppState>,
    request: ::axum::extract::Request,
    next: Next,
) -> Result<Response, crate::TonkWorkerError> {
    if is_profile_scoped_path(request.uri().path())
        && let Some(client) = request.extensions().get::<ClientId>()
    {
        let tonk = state.read().await;
        if !session::client_context_is_current(&tonk, client).await {
            return Err(crate::TonkWorkerError::Conflict(
                "profile changed; reload required".to_string(),
            ));
        }
    }
    Ok(next.run(request).await)
}

/// The token a repository route's `{repo}` segment names the profile's
/// own repository by: `profile:<name>`, as a view's `main@profile:tonk`
/// context spells it. The worker serves one profile, so the name is not
/// consulted.
const PROFILE_ALIAS: &str = "profile:";

/// Whether `repo` is the key of the profile's own repository.
///
/// The profile's repository is a repository like any other, reached by
/// its DID through the same routes. What sets it apart is what may be
/// done to it: a sealed guest never writes it, and its commits belong to
/// no space. Each route that makes that distinction asks here.
pub(crate) async fn names_profile(state: &AppState, repo: &str) -> bool {
    state.read().await.reactor.profile_key() == repo
}

/// The path a repository route is asked for, with a `profile:<name>`
/// repository segment replaced by `key`, the profile's DID, or `None`
/// when the path names no profile alias.
fn resolve_profile_alias(path: &str, key: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/repository/")?;
    let (repo, tail) = match rest.split_once('/') {
        Some((repo, tail)) => (repo, Some(tail)),
        None => (rest, None),
    };
    let alias = repo.starts_with(PROFILE_ALIAS)
        || repo
            .get(..PROFILE_ALIAS.len() + 2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("profile%3a"));
    if !alias {
        return None;
    }
    Some(match tail {
        Some(tail) => format!("/api/repository/{key}/{tail}"),
        None => format!("/api/repository/{key}"),
    })
}

/// Route a request naming the profile's repository by its alias to the
/// repository's own key, before the router matches it. One place knows
/// the alias; every route past it sees a repository key.
async fn profile_alias(
    State(state): State<AppState>,
    mut request: ::axum::extract::Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/api/repository/profile") {
        let key = state.read().await.reactor.profile_key().to_owned();
        if let Some(path) = resolve_profile_alias(request.uri().path(), &key) {
            let rewritten = match request.uri().query() {
                Some(query) => format!("{path}?{query}"),
                None => path,
            };
            if let Ok(uri) = rewritten.parse() {
                *request.uri_mut() = uri;
            }
        }
    }
    next.run(request).await
}

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
    api_router_from_state(Arc::new(crate::StateLock::new(state)))
}

/// Variant of [`api_router`] that also surfaces the wrapped
/// [`AppState`] handle. The worker uses this so it can consult
/// shared state (notably the guest-binding map) outside the
/// request path, before deciding whether to dispatch into the
/// router or pass a fetch through to the network.
pub fn api_router_with_state(state: TonkState) -> (Router, AppState, Arc<LspHub>) {
    let state = Arc::new(crate::StateLock::new(state));
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
        .route(
            "/api/inspect/diagnostics",
            get(diagnostics::read).delete(diagnostics::reset),
        )
        .route("/api/profile", get(profile::get_profile))
        // Register the requesting client's site (per-tab navigation state).
        // The page calls this on load and on each client-side navigation; the
        // SW asserts the tab's `tonk:site` and returns the site id. Reads never
        // stamp — see `router/session.rs`.
        .route("/api/site", post(session::register_site))
        // Per-branch site registration: the branch comes from the URL (like
        // `/query` and `/transact`), not from parsing the document path. A
        // `<tonk-site>` scoped by `<tonk-repository>`/`<tonk-branch>` ancestors
        // posts its path here and renders the returned site entity.
        .route(
            "/api/repository/{repo}/branch/{branch}/site",
            post(session::register_site_on_repo),
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
        .route(
            "/api/repository/{repo}/invites/{target_cid}/revoke",
            post(revoke_invite::revoke),
        )
        .route("/api/repository/{repo}/invites", get(revoke_invite::list))
        // An agent's own space, as the account that made the agent reads and
        // changes it. See `router/agent_space.rs`.
        .route(
            "/api/repository/{repo}/agent",
            get(agent_space::get).post(agent_space::post),
        )
        .route(
            "/api/repository/{repo}/agent/access",
            get(agent_access::access),
        )
        .route(
            "/api/repository/{repo}/agent/chains",
            get(agent_access::chains),
        )
        .route(
            "/api/repository/{repo}/agent/grant",
            post(agent_access::grant),
        );
    // A space's kernel outlines its views for an agent, and keeps the
    // grants its writer is handed.
    #[cfg(target_arch = "wasm32")]
    let router = router
        .route("/api/repository/{repo}/outline", get(view_tree::get))
        // A space's kernel keeps more of the space's chains down to it.
        .route(
            "/api/repository/{repo}/delegations",
            post(space_worker::keep_delegations),
        );
    let router = router
        // Opt-in remote attach — wires a remote (and branch upstream)
        // onto an existing repo, idempotently. See
        // `router/repository.rs::attach_remote`.
        .route(
            "/api/repository/{repo}/remote",
            post(repository::attach_remote),
        )
        // Sync operations
        // The single parameterless drain: the page's idle heartbeat pokes this
        // and the SW's Background-Sync `onsync` reaches the same drain here.
        .route("/api/sync", post(sync::drain))
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
        .route(
            "/api/repository/{repo}/branch/{branch}/evaluate/conditional",
            post(evaluate::evaluate_conditional),
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
        // Host/guest iframe binding. The shell embeds an iframe
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
        // Content-addressed blob bytes: GET serves an entity's bytes; POST
        // ingests a new blob into the branch store and returns its ref.
        // `<tonk-display>` points `<img src>` at the GET form for
        // `tonk:blob` models; `Content-Type` there comes from the asset's
        // `tonk.dialog.asset/media-type` fact, which POST asserts, or the
        // legacy `xyz.tonk.blob/content-type` one older blobs carry.
        // Both stream: neither holds a whole blob in the worker's memory.
        .route(
            "/api/repository/{repo}/branch/{branch}/blob",
            post(blob::upload),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/blob/{entity}",
            get(blob::serve),
        )
        // What the branch's `route/http!` routes answer a path with.
        .route(
            "/api/repository/{repo}/branch/{branch}/http/",
            get(http_route::respond),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/http/{*path}",
            get(http_route::respond),
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
        );
    #[cfg(feature = "persona")]
    let router = persona_routes(router);
    let router = router
        .with_state(state.clone())
        // LSP routes carry their own state (`Extension<LspHub>`) so
        // they don't need to know about `AppState`. Merging keeps
        // the language-server lifetime tied to the worker.
        .merge(lsp_routes)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            profile_context_fence,
        ));
    // A layer runs after its router has matched a route, so the alias is
    // resolved by a router of its own in front, whose only service is the
    // one above.
    let router = Router::new()
        .fallback_service(router)
        .layer(middleware::from_fn_with_state(state, profile_alias));
    (router, lsp_hub)
}

/// `router` with what only the person's own worker serves: their identity,
/// account, customer registration, profiles, and the joins and links they
/// make from their profile. A space's own worker has none of these.
#[cfg(feature = "persona")]
fn persona_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/identity/root",
            get(identity::get).post(identity::save),
        )
        .route("/api/account", get(account::get).delete(account::unlink))
        .route("/api/account/deletion/plan", get(account_deletion::plan))
        .route("/api/account/attach", post(account::link))
        .route("/api/account/display-name", post(account::set_display_name))
        // Customer registration with the same-origin access service.
        .route("/api/customer", get(customer::get_state))
        .route("/api/customer/pending", get(customer::get_pending))
        .route("/api/custody/provision", post(customer::provision_custody))
        .route("/api/custody/queue", post(customer::queue_custody))
        .route("/api/account/devices", get(account_devices::list))
        .route(
            "/api/account/devices/register",
            post(account_devices::register),
        )
        .route("/api/account/devices/revoke", post(account_devices::revoke))
        .route("/api/account/connections", get(agent_connections::list))
        .route(
            "/api/account/connections/{id}/revoke",
            post(agent_connections::revoke),
        )
        // A new agent and its first session, for the OAuth approval page.
        .route("/api/agent/session", post(agent_session::create))
        .route("/api/agent/request/{handle}", get(agent_session::request))
        .route("/api/agent/deny", post(agent_session::deny))
        // The account's agents and what each can reach, for the hub.
        .route("/api/agents", get(agents::list))
        .route("/api/agents/{key}/revoke", post(agents::revoke))
        // Profile roster and switching — every account signed in on this
        // browser has its own profile; these list them, swap the active
        // one, and mint a fresh landing pad for "add account".
        .route("/api/profiles", get(profiles::list))
        .route("/api/profiles/activate", post(profiles::activate))
        .route("/api/profiles/add", post(profiles::add))
        // The profile's own repository has no routes of its own: it is
        // reached through `/api/repository/{repo}/…` like every other,
        // by its DID or the `profile:<name>` alias (see `profile_alias`).
        .route(
            "/api/profile/library",
            post(repository::update_profile_library),
        )
        // Join an invite — creates a fresh replica or refreshes
        // access on an existing one. See `router/join.rs`.
        .route("/api/profile/join", post(join::join))
        .route("/api/view/access", post(view_access::accept))
        .route(
            "/api/local-space-link/approve",
            post(local_space_link::approve),
        )
        .route(
            "/api/local-space-link/describe",
            post(local_space_link::describe),
        )
        .route(
            "/api/local-space-link/provision",
            post(local_space_link::provision),
        )
        .route(
            "/api/local-space-link/complete",
            post(local_space_link::complete),
        )
        .route(
            "/api/migrate/repo-vs-profile",
            get(migration::repo_vs_profile),
        )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod approval_stall_diagnosis {
    use super::repository::profile_library_tests::test_state;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    pub(crate) async fn put_repo(app: &Router, label: &str) -> String {
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
    #[test]
    fn local_identity_remains_responsive_during_stalled_sync_and_pull() {
        // Unoptimized native network/authority futures need more stack than
        // the test harness default. Keep this fixture independent of CI env.
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(stalled_sync_and_pull());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn stalled_sync_and_pull() {
        use super::repository::{
            BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
        };
        use dialog_remote_ucan::UcanAddress;
        use dialog_repository::SiteAddress;
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/ucan/", listener.local_addr().unwrap());
        let (app, state, _lsp) = super::api_router_with_state(test_state().await);
        let repo = put_repo(&app, "approval-stall").await;
        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(&endpoint))),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let syncing = super::sync::sync(
            axum::extract::State(state.clone()),
            axum::extract::Path(super::sync::SyncPath {
                repo: repo.clone(),
                branch: "main".into(),
            }),
        );
        tokio::pin!(syncing);
        let socket = tokio::select! {
            accepted = listener.accept() => accepted.unwrap().0,
            result = &mut syncing => panic!("sync returned before reaching remote: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("sync did not reach remote"),
        };
        // Keep the accepted connection open without answering: remote IO is pending.
        assert!(state.try_read().is_ok(), "sync alone allows local readers");
        let mut pulling = Box::pin(super::sync::pull(
            axum::extract::State(state.clone()),
            axum::extract::Path(super::sync::SyncPath {
                repo,
                branch: "main".into(),
            }),
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut pulling)
                .await
                .is_err()
        );
        let identity = super::identity::get(axum::extract::State(state.clone()));
        let outcome = tokio::time::timeout(Duration::from_millis(100), identity).await;
        let blocked = outcome.is_err();
        drop(pulling);
        let recovered = tokio::time::timeout(
            Duration::from_secs(1),
            super::identity::get(axum::extract::State(state.clone())),
        )
        .await;
        assert!(
            recovered.is_ok(),
            "identity recovers after cancelling only the queued pull"
        );
        eprintln!(
            "remote sync pending; concurrent pull blocked identity={blocked}; identity after cancellation=true"
        );
        drop(socket);
        assert!(
            !blocked,
            "queued writer behind remote sync blocks local identity reads"
        );
    }

    /// Requests that commit, export, import or hydrate a starter image
    /// answer while a sync holds its read guard across a round trip
    /// the remote never answers.
    ///
    /// Each of these used to take the state's write guard, which waits for
    /// every reader to finish. With a sync in flight that is the whole
    /// round trip, and while the writer waits, every request after it waits
    /// too. The remote here accepts the connection and never answers, so a
    /// route that still took the write guard would never answer at all.
    #[test]
    fn writing_routes_answer_while_a_sync_waits_on_its_remote() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(writing_routes_during_a_stalled_sync());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn writing_routes_during_a_stalled_sync() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (app, state, _lsp) = super::api_router_with_state(test_state().await);
        let (repo, mut syncing, _sockets) =
            a_sync_waiting_on_its_remote(&app, &state, &listener, "stalled-sync-writers").await;

        let branch = format!("/api/repository/{repo}/branch/main");
        let requests: Vec<(&str, String, &str, Vec<u8>)> = vec![
            (
                "POST",
                format!("{branch}/claim/assert/test:stalled/test/name"),
                "text/plain",
                b"asserted".to_vec(),
            ),
            (
                "POST",
                format!("{branch}/claim/retract/test:stalled/test/name"),
                "text/plain",
                b"asserted".to_vec(),
            ),
            (
                "POST",
                format!("{branch}/blob"),
                "text/plain",
                b"uploaded while a sync waits".to_vec(),
            ),
            (
                "GET",
                format!("{branch}/blob/blob:CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG"),
                "",
                Vec::new(),
            ),
            ("GET", format!("{branch}/export"), "", Vec::new()),
        ];
        for (method, uri, content_type, body) in requests {
            let answered = tokio::time::timeout(
                Duration::from_secs(10),
                send(&app, method, &uri, content_type, body),
            )
            .await;
            let Ok((status, body)) = answered else {
                panic!("{method} {uri} waited on the sync's round trip");
            };
            assert!(
                status.is_success(),
                "{method} {uri}: {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        // The export's rows back in, onto a branch the upload above moved
        // through another handle than the import's. The asset facts dialog
        // records itself are left out: an import refuses its namespace.
        let (_, exported) = send(&app, "GET", &format!("{branch}/export"), "", Vec::new()).await;
        let exported = String::from_utf8(exported)
            .unwrap()
            .lines()
            .filter(|row| !row.starts_with("dialog."))
            .map(|row| format!("{row}\n"))
            .collect::<String>()
            .into_bytes();
        let imported = tokio::time::timeout(
            Duration::from_secs(10),
            send(
                &app,
                "POST",
                &format!("{branch}/import"),
                "text/csv",
                exported,
            ),
        )
        .await
        .expect("an import waited on the sync's round trip");
        assert!(
            imported.0.is_success(),
            "import: {}: {}",
            imported.0,
            String::from_utf8_lossy(&imported.1)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut syncing)
                .await
                .is_err(),
            "the sync is still waiting on its remote"
        );
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> (StatusCode, Vec<u8>) {
        let mut request = Request::builder().uri(uri).method(method);
        if !content_type.is_empty() {
            request = request.header("content-type", content_type);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, body.to_vec())
    }

    /// A space with a remote that accepts connections and never answers,
    /// and a sync of it driven until it waits on that remote. The sockets
    /// are returned so the remote stays silent while the test runs.
    async fn a_sync_waiting_on_its_remote(
        app: &Router,
        state: &super::AppState,
        listener: &tokio::net::TcpListener,
        label: &str,
    ) -> (
        String,
        std::pin::Pin<
            Box<
                impl std::future::Future<
                    Output = Result<axum::Json<super::sync::SyncResponse>, crate::TonkWorkerError>,
                >,
            >,
        >,
        Vec<tokio::net::TcpStream>,
    ) {
        use super::repository::{
            BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
        };
        use dialog_remote_ucan::UcanAddress;
        use dialog_repository::SiteAddress;
        use std::time::Duration;

        let endpoint = format!("http://{}/ucan/", listener.local_addr().unwrap());
        let repo = put_repo(app, label).await;
        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(&endpoint))),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let mut syncing = Box::pin(super::sync::sync(
            axum::extract::State(state.clone()),
            axum::extract::Path(super::sync::SyncPath {
                repo: repo.clone(),
                branch: "main".into(),
            }),
        ));
        let mut sockets = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        // Past its first connection, a sync that has gone quiet for half a
        // second is waiting on the remote, not working locally.
        loop {
            tokio::select! {
                accepted = listener.accept() => sockets.push(accepted.unwrap().0),
                result = &mut syncing => panic!("sync returned before reaching its remote: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if !sockets.is_empty() {
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "the sync never reached its remote"
                    );
                }
            }
        }
        (repo, syncing, sockets)
    }

    /// A sync waiting on its remote holds no guard over the state, so a
    /// route that takes the write guard answers, and so does every request
    /// after it. Creating a space is such a route. When a sync held a read
    /// guard across its round trip, the create waited for the remote, and
    /// with a write-preferring lock every request behind it waited too: a
    /// remote that never answers froze the worker.
    #[test]
    fn a_writer_answers_while_a_sync_waits_on_its_remote() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(a_writer_during_a_stalled_sync());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn a_writer_during_a_stalled_sync() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (app, state, _lsp) = super::api_router_with_state(test_state().await);
        let (_repo, mut syncing, _sockets) =
            a_sync_waiting_on_its_remote(&app, &state, &listener, "stalled-under-a-writer").await;

        assert!(
            state.try_write().is_ok(),
            "a sync waiting on its remote holds a guard over the state"
        );
        let created = tokio::time::timeout(
            Duration::from_secs(10),
            put_repo(&app, "created-while-a-sync-waits"),
        )
        .await;
        assert!(
            created.is_ok(),
            "creating a space waited on another space's sync"
        );
        let profile = tokio::time::timeout(
            Duration::from_secs(10),
            super::profile::get_profile(axum::extract::State(state.clone())),
        )
        .await;
        assert!(
            profile.is_ok(),
            "a read after the create waited on the sync"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut syncing)
                .await
                .is_err(),
            "the sync is still waiting on its remote"
        );
    }

    /// Creating a space provisions it with the access service, a round trip
    /// to that service. The create used to make it under the write guard,
    /// so every other request waited on the service, up to its timeout. It
    /// now provisions with no guard held: while the service is silent, a
    /// writer could take the guard and a read answers.
    #[test]
    fn creating_a_space_holds_no_guard_while_it_provisions() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(creation_during_a_stalled_provision());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Make `tonk` an active customer served by `provider`, so a create
    /// provisions there.
    async fn served_by(tonk: &crate::worker::TonkState, provider: &str) {
        use dialog_credentials::Ed25519Signer;
        use tonk_schema::prelude::DidExt as _;

        let root = Ed25519Signer::import(&[79; 32]).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root, &tonk.profile.did())
            .await
            .unwrap();
        super::identity::persist_root(
            tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "provisioning-stall".into(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .unwrap();
        let root = super::identity::local_root(tonk).await.unwrap().root_did;
        let record = tonk_account::AccountProviderRecord::attach(provider, 1).unwrap();
        tonk.profile
            .secrets()
            .site(tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE)
            .save(record.encode().unwrap())
            .perform(&tonk.profile)
            .await
            .unwrap();
        tonk.reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .transaction()
            .assert(tonk_schema::AccountRegistered::new(
                root.this(),
                "stalled@example.test".into(),
                provider.into(),
                1,
            ))
            .assert(tonk_schema::AccountActive::new(root.this(), 2))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert!(super::customer::is_active(tonk).await);
    }

    async fn creation_during_a_stalled_provision() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = format!("http://{}/ucan/", listener.local_addr().unwrap());
        let tonk = test_state().await;
        served_by(&tonk, &provider).await;
        let (app, state, _lsp) = super::api_router_with_state(tonk);

        let mut creating = Box::pin(put_repo(&app, "provisioned-while-the-service-stalls"));
        let socket = tokio::select! {
            accepted = listener.accept() => accepted.unwrap().0,
            _ = &mut creating => panic!("the create answered without provisioning"),
            _ = tokio::time::sleep(Duration::from_secs(10)) => {
                panic!("the create never reached the access service")
            }
        };

        assert!(
            state.try_write().is_ok(),
            "a create waiting on the access service holds a guard over the state"
        );
        let profile = tokio::time::timeout(
            Duration::from_secs(2),
            super::profile::get_profile(axum::extract::State(state.clone())),
        )
        .await;
        assert!(profile.is_ok(), "a read waited on a create's provisioning");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut creating)
                .await
                .is_err(),
            "the create is still waiting on the access service"
        );

        // A service that hangs up leaves the space local-only: the create
        // still answers.
        drop(socket);
        drop(listener);
        tokio::time::timeout(Duration::from_secs(30), creating)
            .await
            .expect("the create answers once the service hangs up");
    }

    /// The account panel's `GET /api/customer` asks the access service for
    /// the customer's live status. While that service is silent, the route
    /// holds no guard over the state.
    #[test]
    fn the_customer_probe_holds_no_guard_while_the_service_answers() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(customer_probe_during_a_stalled_service());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn customer_probe_during_a_stalled_service() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let tonk = test_state().await;
        served_by(&tonk, &format!("{origin}ucan/")).await;
        let (_app, state, _lsp) = super::api_router_with_state(tonk);

        let mut probing = Box::pin(super::customer::get_state(
            axum::extract::State(state.clone()),
            axum::Extension(crate::axum::RequestOrigin::parse(&origin).unwrap()),
        ));
        let socket = tokio::select! {
            accepted = listener.accept() => accepted.unwrap().0,
            _ = &mut probing => panic!("the probe answered without asking the service"),
            _ = tokio::time::sleep(Duration::from_secs(10)) => {
                panic!("the probe never reached the service")
            }
        };
        assert!(
            state.try_write().is_ok(),
            "the customer probe holds a guard over the state while the service answers"
        );
        drop(socket);
        drop(listener);
        let _ = tokio::time::timeout(Duration::from_secs(30), probing)
            .await
            .expect("the probe answers once the service hangs up");
    }

    /// Turning sync on for a space provisions it with the access service, a
    /// round trip. While that service is silent, the attach holds no guard
    /// over the state.
    #[test]
    fn enabling_sync_holds_no_guard_while_it_provisions() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(enabling_sync_during_a_stalled_provision());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn enabling_sync_during_a_stalled_provision() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = format!("http://{}/ucan/", listener.local_addr().unwrap());
        let (app, state, _lsp) = super::api_router_with_state(test_state().await);
        // Created before the account is served, so it is local-only.
        let key = put_repo(&app, "synced-while-the-service-stalls").await;
        served_by(&*state.read().await, &provider).await;

        let mut enabling = Box::pin(super::repository::enable_sync_inner(
            &state, &key, &provider,
        ));
        let socket = tokio::select! {
            accepted = listener.accept() => accepted.unwrap().0,
            _ = &mut enabling => panic!("sync was turned on without asking the service"),
            _ = tokio::time::sleep(Duration::from_secs(10)) => {
                panic!("turning sync on never reached the service")
            }
        };
        assert!(
            state.try_write().is_ok(),
            "turning sync on holds a guard over the state while the service answers"
        );
        drop(socket);
        drop(listener);
        let _ = tokio::time::timeout(Duration::from_secs(30), enabling)
            .await
            .expect("turning sync on answers once the service hangs up");
    }

    /// What used to keep a sync from writing into a space being removed
    /// was the write guard the removal took, which waited out every sync.
    /// A sync now holds no guard, so the space's own gate does it: removing
    /// the space waits for that space's sync, and only for it. Removing
    /// another space, and reads, go on meanwhile.
    #[test]
    fn removing_a_space_waits_for_its_own_sync_only() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(removal_during_a_stalled_sync());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn removal_during_a_stalled_sync() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (app, state, _lsp) = super::api_router_with_state(test_state().await);
        let other = put_repo(&app, "removed-beside-a-sync").await;
        let (repo, syncing, sockets) =
            a_sync_waiting_on_its_remote(&app, &state, &listener, "removed-while-syncing").await;

        let other: dialog_varsig::Did = other.parse().unwrap();
        let removed = tokio::time::timeout(
            Duration::from_secs(10),
            super::repository::remove_space_inner(&state, &other),
        )
        .await;
        assert!(
            matches!(removed, Ok(Ok(()))),
            "removing another space waited on this one's sync: {removed:?}"
        );

        let subject: dialog_varsig::Did = repo.parse().unwrap();
        let mut removing = Box::pin(super::repository::remove_space_inner(&state, &subject));
        assert!(
            tokio::time::timeout(Duration::from_millis(500), &mut removing)
                .await
                .is_err(),
            "the space was removed while its sync was still in flight"
        );
        let profile = tokio::time::timeout(
            Duration::from_secs(10),
            super::profile::get_profile(axum::extract::State(state.clone())),
        )
        .await;
        assert!(
            profile.is_ok(),
            "a read waited on a removal waiting on a sync"
        );

        drop(syncing);
        drop(sockets);
        let removed = tokio::time::timeout(Duration::from_secs(10), &mut removing).await;
        assert!(
            matches!(removed, Ok(Ok(()))),
            "the removal did not finish once the sync was gone: {removed:?}"
        );
    }
}
