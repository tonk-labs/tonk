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
use tokio::sync::RwLock;

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
mod account_deletion;
mod account_journey;
mod ceremony;
pub(crate) mod customer;
mod email_status;
mod link_timing;
/// The panel that adds an account to this profile, stage by stage.
pub(crate) mod registration;
mod sign_in_via;

pub(crate) mod account_state;
pub use account_state::AccountKeys;

mod http;

pub(crate) mod adopt;
/// Getting the account's encryption key onto a device that needs it.
pub(crate) mod custody;
/// Accreditation: rotate the onboarding account's custody to the passkey
/// account, then retire it.
pub(crate) mod rotation;

/// Acting as the account on a kernel, once the person lets it.
#[cfg(target_arch = "wasm32")]
mod authorize;
pub(crate) mod join;
/// Signing in on a kernel: the provisional persona merges into the
/// permanent one.
#[cfg(target_arch = "wasm32")]
mod merge;
#[cfg(target_arch = "wasm32")]
pub use authorize::{authorized, delegate_asked, unauthorized};
pub(crate) mod public_access;
mod publication;
/// Reaching a space's own worker from the person's profile.
pub(crate) mod space_directory;
pub(crate) mod space_reach;
/// A space's own worker: the delegation it holds, and the space it mounts.
#[cfg(target_arch = "wasm32")]
pub(crate) mod space_worker;
pub use join::{JoinRequest, JoinResponse};
#[cfg(target_arch = "wasm32")]
pub use merge::{failed, merged, refused};
mod local_space_link;
/// What a space's own worker asks of the persona, on a kernel.
#[cfg(target_arch = "wasm32")]
pub mod profile_asks;

pub(crate) mod account_devices;

mod create_invite;
pub use create_invite::{CreateInviteRequest, CreateInviteResponse};

pub(crate) mod agent_connections;
pub(crate) mod revoke_invite;

/// Space membership management: admins and removals, as commands.
mod members;

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

pub(crate) mod profiles;

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

mod transfer;
pub use transfer::ImportResponse;

mod host;
pub use host::{ClientId, ViewBinding, ViewBindings};

mod blob;

mod migration;

mod interpret;
mod navigate;
mod site_request;
mod site_select;

mod command;
pub use command::{CommandEnv, CommandOrigin, CommandProviders, command_providers, dispatch};

#[cfg(test)]
mod intent;
#[cfg(test)]
mod route_table;
#[cfg(test)]
mod wire_compat;

/// Shared application state containing profile and operator.
pub type AppState = Arc<RwLock<TonkState>>;

fn is_profile_scoped_path(path: &str) -> bool {
    path.starts_with("/api/") && !matches!(path, "/api/identify" | "/api/health")
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
        .route("/api/profile", get(profile::get_profile))
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
        // Join an invite — creates a fresh replica or refreshes
        // access on an existing one. See `router/join.rs`.
        .route("/api/profile/join", post(join::join))
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
        .route("/api/account/connections", get(agent_connections::list))
        .route(
            "/api/account/connections/{id}/revoke",
            post(agent_connections::revoke),
        )
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
        )
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
