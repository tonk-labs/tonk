//! API router configuration and handlers.

use std::sync::Arc;

use ::axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    routing::post,
    routing::put,
};
use tokio::sync::RwLock;

use crate::worker::TonkState;

/// Whether a newer service worker is installed and WAITING to take over —
/// i.e. whether this worker is retiring.
///
/// Read live from the registration rather than latched at `updatefound`,
/// so it is self-healing: an update that never activates (a failed
/// install, a canceled upgrade) clears the `waiting` slot and this
/// worker goes back to serving streams normally. A latch would leave it
/// permanently refusing.
///
/// Every route that opens a LONG-LIVED response must consult this. An
/// SSE body is a fetch event that never settles, and the spec keeps a
/// worker alive while any of its fetch events are in flight — so a
/// single stream opened after the successor started installing re-pins
/// this worker and parks the new one in `waiting` indefinitely. That is
/// the "reloading doesn't help, it's still the old version" symptom:
/// reloads land on the old ACTIVE worker, which is exactly what's
/// keeping the new one out.
///
/// `false` off-wasm and whenever the registration is unreadable — a
/// wrongly refused stream would starve consumers, a wrongly opened one
/// only delays an update.
pub(crate) fn update_pending() -> bool {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;
        return js_sys::global()
            .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
            .map(|scope| scope.registration().waiting().is_some())
            .unwrap_or(false);
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    false
}

mod claim;
pub use claim::{AssertPath, AssertResponse, ClaimQuery, ClaimResponse, QueryResponse};

mod account;
mod account_deletion;
pub(crate) mod customer;
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
mod email_status;

pub(crate) mod account_state;
pub use account_state::AccountKeys;

mod http;

pub(crate) mod adopt;
/// Getting the account's encryption key onto a device that needs it.
pub(crate) mod custody;
/// Accreditation: rotate the onboarding account's custody to the passkey
/// account, then retire it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) mod rotation;

mod join;
pub use join::{JoinRequest, JoinResponse};

pub(crate) mod account_devices;

mod create_invite;
pub use create_invite::{CreateInviteRequest, CreateInviteResponse};

mod revoke_invite;

/// Space membership management: admins and removals, as commands.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod members;

pub mod inspect;
pub use inspect::{BranchStatusResponse, RemoteBranchStatusResponse, RemoteStatusResponse};

mod repository;
pub use repository::{
    BranchConfiguration, MemberInfo, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    UpstreamConfiguration, bootstrap_profile,
};

mod sync;
pub use dialog_repository::Revision;
pub use sync::{
    SyncQueue, SyncResponse, SyncStatusResponse, branches_to_sync, drain_sync, sync_repository,
};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use sync::{is_sync_enabled, mark_offline};
// Re-exported so API consumers (the UI) can name the state without
// depending on `tonk-schema` directly.
pub use tonk_schema::SyncState;

mod identify;
pub use identify::IdentifyResponse;

pub(crate) mod identity;

pub mod lsp;
pub use lsp::LspHub;

mod lsp_env;

mod profile;
pub use profile::{ProfileInfo, SpaceEntry};

pub(crate) mod profiles;

mod profile_name;

mod evaluate;
pub use evaluate::{CommitSummary, EvaluatePath, EvaluateResponse, QueryMatchBlock, QueryResult};

mod query;
pub use query::QueryPath;

// Level 0 routing lives in `tonk-schema` (shared with the UI); re-export it so
// the SW's routing/containment code reads it locally.
pub use tonk_schema::{DEFAULT_BRANCH, SpaceRef, parse_space};

mod session;
pub use session::{ClientRegistry, ClientState, SiteResponse};

mod transact;
pub use transact::{ProfileTransactPath, TransactPath, TransactResponse};

mod transfer;
pub use transfer::ImportResponse;

pub mod bridge;
pub use bridge::BridgeRegistry;

mod host;
pub use host::{ClientId, ViewBinding, ViewBindings};

mod blob;

mod migration;

mod navigate;

mod command;
pub use command::{CommandEnv, CommandOrigin, command_registry, dispatch};

#[cfg(test)]
mod route_table;
#[cfg(test)]
mod wire_compat;

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

/// Largest accepted `POST …/blob` body. The handler buffers the whole
/// upload in service-worker memory before writing it to the blob store,
/// so this caps that buffer, not just the wire size. Keep this conservative
/// until the browser upload and remote sync paths stream end to end.
pub const BLOB_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;

/// The build header the page sends on every `/api/*` request.
const BUILD_HEADER: &str = "x-tonk-build";

/// This worker's stamped build id. Only the browser build has one —
/// natively there is no service worker to be out of step with, so the
/// handshake is inert there.
fn current_build_id() -> Option<String> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        crate::cache::current_build_id()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    None
}

/// Whether this request changes state, and so must be refused when the
/// page is from another build.
///
/// Deliberately NOT "is it a POST": the data plane posts to read as
/// well (a `query` carries its body, and a subscription is a `POST`
/// that streams), so method alone would refuse every read too. Match
/// the write routes by path instead.
fn is_mutating(request: &axum::extract::Request) -> bool {
    let path = request.uri().path();
    path.ends_with("/transact") || path.ends_with("/evaluate")
}

/// Whether a request should be refused as coming from a stale page,
/// returning the pair to report when it should.
///
/// Split out from the middleware so the decision is testable without a
/// service-worker global. Both ids must be present and differ: an
/// absent header (a sealed guest, an older page) and an unstamped
/// worker (dev, native) are both "cannot classify", and a request that
/// cannot be classified must never be blocked.
fn stale_build(ours: Option<&str>, theirs: Option<&str>) -> Option<(String, String)> {
    let (ours, theirs) = (ours?, theirs?);
    (ours != theirs).then(|| (ours.to_owned(), theirs.to_owned()))
}

/// Refuse a request from a page built against a different version of
/// this worker's HTTP surface.
///
/// `skipWaiting` + `clients.claim()` swap the worker underneath running
/// pages, so a page's wasm and the worker answering it can come from
/// different builds. Storage is engineered for that overlap (CAS over
/// content-addressed blocks), but the HTTP surface is not versioned: a
/// renamed route or a changed DTO shows up as a confusing 404 or parse
/// error in a page that has no idea it is stale.
///
/// A structured `409` turns that mystery into a reload prompt. Absent
/// or unknown build ids pass through untouched — the header is a hint,
/// and a request that cannot be classified must not be blocked (a
/// sealed guest, an older page, or a context with no stamp at all).
async fn reject_stale_build(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let theirs = request
        .headers()
        .get(BUILD_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    // Only ever refuse a WRITE. A stale page reading is harmless — it
    // gets data it can render — but a stale page that cannot write is
    // also a page that cannot subscribe, and killing its subscriptions
    // leaves it frozen with no way to notice. Observed in practice: a
    // `409` on `POST …/query` (a subscription, despite the verb) took
    // out the page's live updates during an ordinary worker swap.
    //
    // The point of the handshake is to explain a confusing failure, not
    // to manufacture one. Reads pass; the prompt still reaches the user
    // on their next write.
    if !is_mutating(&request) {
        return next.run(request).await;
    }

    let Some((ours, theirs)) = stale_build(current_build_id().as_deref(), theirs.as_deref()) else {
        return next.run(request).await;
    };

    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": {
                "kind": "stale-build",
                "message": "this page was built against a different worker version",
                "page": theirs,
                "worker": ours,
            }
        })),
    )
        .into_response()
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
        .route("/api/account/delete", post(account_deletion::delete))
        .route(
            "/api/account/spaces/delete",
            post(account_deletion::delete_space),
        )
        .route("/api/account/attach", post(account::link))
        .route("/api/account/display-name", post(account::set_display_name))
        // Customer registration with the same-origin access service.
        .route("/api/customer", get(customer::get_state))
        .route("/api/customer/pending", get(customer::get_pending))
        .route("/api/custody/provision", post(customer::provision_custody))
        .route("/api/custody/queue", post(customer::queue_custody))
        .route("/api/account/devices", get(account_devices::list))
        .route("/api/account/summary", get(account_devices::summary))
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
            "/api/profile/branch/{branch}/site",
            post(session::register_site_on_profile),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/site",
            post(session::register_site_on_repo),
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
        .route(
            "/api/repository/{repo}/invites/{target_cid}/revoke",
            post(revoke_invite::revoke),
        )
        .route("/api/repository/{repo}/invites", get(revoke_invite::list))
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
        // Content-addressed blob bytes: GET serves an entity's bytes; POST
        // ingests a new blob into the branch store and returns its ref.
        // `<tonk-display>` points `<img src>` at the GET form for
        // `tonk:blob` models; `Content-Type` there comes from the blob's
        // `xyz.tonk.blob/content-type` fact, which POST asserts.
        // The upload body is buffered whole in the service worker (no
        // streaming yet), so the limit is a deliberate ceiling rather than
        // axum's 2 MiB default — which real image files routinely exceed.
        .route(
            "/api/repository/{repo}/branch/{branch}/blob",
            post(blob::upload).layer(DefaultBodyLimit::max(BLOB_UPLOAD_LIMIT)),
        )
        .route(
            "/api/repository/{repo}/branch/{branch}/blob/{entity}",
            get(blob::serve),
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
        // Version handshake. Applied to the whole `/api` surface rather
        // than per-route: the point is to catch a route or DTO this
        // page does not know about, which by definition can be any of
        // them.
        .layer(axum::middleware::from_fn(reject_stale_build));
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

    use dialog_credentials::Ed25519Signer;
    use dialog_operator::Profile;
    use dialog_repository::RepositoryExt as _;
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

    /// Failure-safe replacement for one service-worker global used by a test.
    pub(crate) struct GlobalPropertyGuard {
        global: js_sys::Object,
        name: wasm_bindgen::JsValue,
        previous: Option<wasm_bindgen::JsValue>,
    }

    impl GlobalPropertyGuard {
        /// Replace `globalThis[name]` until this guard is dropped.
        pub(crate) fn replace(name: &str, value: &wasm_bindgen::JsValue) -> Self {
            let global = js_sys::global();
            let name = wasm_bindgen::JsValue::from_str(name);
            let previous = js_sys::Reflect::has(&global, &name)
                .expect("global property can be inspected")
                .then(|| {
                    js_sys::Reflect::get(&global, &name).expect("global property can be read")
                });
            js_sys::Reflect::set(&global, &name, value).expect("global property can be replaced");
            Self {
                global,
                name,
                previous,
            }
        }
    }

    impl Drop for GlobalPropertyGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(previous) => {
                    let _ = js_sys::Reflect::set(&self.global, &self.name, previous);
                }
                None => {
                    let _ = js_sys::Reflect::delete_property(&self.global, &self.name);
                }
            }
        }
    }

    #[dialog_common::test]
    async fn global_property_guard_restores_and_deletes_after_an_early_error() {
        fn replace_then_fail(name: &str) -> Result<(), ()> {
            let _replacement =
                GlobalPropertyGuard::replace(name, &wasm_bindgen::JsValue::from_str("temporary"));
            Err(())
        }

        let global = js_sys::global();
        let existing = "__tonkGuardCleanupExisting";
        let _baseline =
            GlobalPropertyGuard::replace(existing, &wasm_bindgen::JsValue::from_str("original"));
        assert_eq!(replace_then_fail(existing), Err(()));
        assert_eq!(
            js_sys::Reflect::get(&global, &wasm_bindgen::JsValue::from_str(existing))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("original")
        );

        let missing = wasm_bindgen::JsValue::from_str("__tonkGuardCleanupMissing");
        js_sys::Reflect::delete_property(&global, &missing).unwrap();
        assert_eq!(replace_then_fail("__tonkGuardCleanupMissing"), Err(()));
        assert!(
            !js_sys::Reflect::has(&global, &missing).unwrap(),
            "a helper global absent before replacement must be deleted"
        );
    }

    /// A random id minted once per test *process*, mixed into every profile
    /// name so two runs never collide on storage a shared browser profile
    /// kept between them.
    fn session_nonce() -> u32 {
        use std::sync::OnceLock;
        static NONCE: OnceLock<u32> = OnceLock::new();
        *NONCE.get_or_init(rand::random::<u32>)
    }

    /// Creates a test state with the default storage backend.
    ///
    /// The state has a profile and operator but *no* repository —
    /// tests that need one call [`put_repo`] with a display label and
    /// use the minted routing key it returns. Every create mints a
    /// fresh identity for the repos it makes, but the profile itself
    /// is durable IndexedDB state keyed by name: each call mints its
    /// own unique profile name so tests that rename or restamp the
    /// profile never bleed into one another.
    ///
    /// The sequence number alone is unique only *within* a run —
    /// `test-tonk-3` is whichever test happened to run third — so a
    /// runner that reuses a browser profile (safaridriver, a persistent
    /// Chrome user-data-dir) would hand run N's leftover IndexedDB to
    /// run N+1's third test, reviving the order dependence in cross-run
    /// form. `wasm-bindgen-test-runner`'s throwaway Chrome profile hides
    /// that today; the [`session_nonce`] makes it unconditional.
    pub async fn test_state_without_root() -> TonkState {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let profile_name = format!(
            "test-tonk-{}-{}",
            session_nonce(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );

        crate::patch_idb_versionchange();
        let storage = Storage::<DefaultSpace>::default();
        let profile = Profile::open(&profile_name)
            .perform(&storage)
            .await
            .expect("Failed to create test profile");

        let session = crate::session::open(&profile, &storage)
            .await
            .expect("Failed to open a test signing session");

        let reactor = crate::Reactor::new(profile.clone());
        // The registry mirrors production shape — the state's own profile
        // is the registry profile, exactly as `Registry::device()` signs
        // as `tonk` until the first rotation. Uniquely named per state,
        // so tests neither collide with each other nor touch the real
        // registry, while rotated/activated profiles still resolve in the
        // same directory the test profile itself lives in.
        let registry = crate::device::Registry {
            profile: profile_name.clone(),
            directory: dialog_effects::storage::Directory::Profile,
        };
        TonkState {
            profile,
            operator: session.operator,
            storage,
            session_expires_at: session.expires_at,
            profile_name,
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            sync_queue: Default::default(),
            commands: super::command_registry(),
            clients: Default::default(),
            account_keys: Default::default(),
            registry,
        }
    }

    /// The root seed for a test profile, derived from its name.
    ///
    /// Per-profile rather than one shared constant, because the account
    /// repository's routing key IS the root's — so every profile sharing a
    /// root shares one account repository, and its storage is not scoped by
    /// profile the way a space's is. Two tests that link descriptors naming
    /// different remotes then fight over the same mount, and the second one
    /// to run reads the first one's remote and refuses as a conflict. That
    /// is invisible until the ordering shifts, which is exactly the failure
    /// [`session_nonce`] exists to prevent one layer down.
    ///
    /// A fold rather than a hash: no dependency, deterministic, and it mixes
    /// every byte of the name — which is all that separating test profiles
    /// requires.
    pub(crate) fn test_root_seed(profile_name: &str) -> [u8; 32] {
        let mut seed = [42u8; 32];
        for (index, byte) in profile_name.as_bytes().iter().enumerate() {
            seed[index % 32] ^= byte.rotate_left((index % 8) as u32);
        }
        seed
    }

    /// Create an isolated test state with a stable local root grant and no
    /// account attached to it.
    ///
    /// The shape a device is in between provisioning a root and finishing
    /// sign-up. Only the tests that assert a durable operation refuses want
    /// it; everything else wants [`test_state`], because production never
    /// creates a root without an account around it.
    pub async fn test_state_without_account() -> TonkState {
        let state = test_state_without_root().await;
        persist_test_root(&state).await;
        state
    }

    /// Persist the test root on `state`, the way a creation or unlock
    /// ceremony does: the `root -> device` grant, the recipient custodied
    /// seeds are sealed to, and that recipient published on profile main.
    /// Returns the root DID.
    pub(crate) async fn persist_test_root(state: &TonkState) -> dialog_varsig::Did {
        let root = Ed25519Signer::import(&test_root_seed(&state.profile_name))
            .await
            .unwrap();
        let root_did = root.did();
        let grant = tonk_identity::delegation::mint_device_delegation(root, &state.profile.did())
            .await
            .unwrap();
        // What a creation or unlock ceremony hands back with the root, and
        // what the account sweep then publishes: the recipient custodied
        // seeds are sealed to. Published here directly, since the fixture
        // has no account branch to sweep.
        let recipient = tonk_identity::envelope::AccountSecret::from_bytes(
            zeroize::Zeroizing::new(test_root_seed(&state.profile_name)),
        )
        .secret()
        .did();
        super::identity::persist_root(
            state,
            tonk_worker_api::SaveRootRequest {
                credential_id: "test-credential".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();
        state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .transaction()
            .assert(tonk_schema::AccountSealedInbox::new(
                root_did.this(),
                recipient.this(),
            ))
            .commit()
            .perform(&state.operator)
            .await
            .expect("the fixture publishes the account's encryption key");
        root_did
    }

    /// Create an isolated test state with a stable local root grant and an
    /// account attached to it — a signed-in device.
    pub async fn test_state() -> TonkState {
        let state = test_state_without_account().await;
        super::account::attach_test_account(&state).await.unwrap();
        state
    }

    /// Query all `Membership` rows on `repo`'s content branch.
    pub(crate) async fn content_memberships(
        state: &super::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::Membership> {
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::{Branch, Repository, RepositoryExt as _};
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");
        content
            .query()
            .select(Query::<tonk_schema::Membership> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                member: Term::var("member"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("membership query")
    }

    /// Query all `Invitation` rows on `repo`'s content branch.
    pub(crate) async fn content_invitations(
        state: &super::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::Invitation> {
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::{Branch, Repository, RepositoryExt as _};
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");
        content
            .query()
            .select(Query::<tonk_schema::Invitation> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                inviter: Term::var("inviter"),
                audience: Term::var("audience"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("invitation query")
    }

    /// Query all `InvitedVia` rows on `repo`'s content branch.
    pub(crate) async fn content_invited_via(
        state: &super::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::InvitedVia> {
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::{Branch, Repository, RepositoryExt as _};
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");
        content
            .query()
            .select(Query::<tonk_schema::InvitedVia> {
                this: Term::var("this"),
                invitation: Term::var("invitation"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("invited-via query")
    }

    /// Query all `MemberRole` rows on `repo`'s content branch.
    pub(crate) async fn content_member_roles(
        state: &super::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::MemberRole> {
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::{Branch, Repository, RepositoryExt as _};
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");
        content
            .query()
            .select(Query::<tonk_schema::MemberRole> {
                this: Term::var("this"),
                role: Term::var("role"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("member-role query")
    }

    /// Query all `MemberName` rows on `repo`'s content branch.
    pub(crate) async fn content_member_names(
        state: &super::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::MemberName> {
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::{Branch, Repository, RepositoryExt as _};
        let tonk = state.read().await;
        let repository: Repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let content: Branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .expect("content branch opens");
        content
            .query()
            .select(Query::<tonk_schema::MemberName> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("member-name query")
    }

    /// Creates a test repository via `PUT /api/repository/{label}` and
    /// returns its minted routing key.
    ///
    /// `label` is only a display name now — the repository's identity is
    /// a freshly minted `did:key`, and the routing key returned here (the
    /// DID suffix from the 201 `RepositoryInfo`) is what every subsequent
    /// request must address. Each PUT always creates, so runs are
    /// independent without name juggling.
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

    /// The stable code the page routes the account gate off, for both shapes
    /// of "not signed in": no root at all, and a root with no account behind
    /// it. Neither may create a space — one that exists without an account is
    /// local-only and never backed up, and nothing later would say so.
    /// A space creates before any account exists, delegated to the most
    /// durable key the profile holds (plan/Account model.md §2): the
    /// device key when there is no root, the root when there is one.
    #[dialog_common::test]
    async fn it_creates_a_space_before_any_account_exists() {
        let (app, state, _lsp) = super::api_router_with_state(test_state_without_root().await);
        let key = put_repo(&app, "pre-account").await;
        {
            let tonk = state.read().await;
            let repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            let prefix = super::repository::space_root_prefix(&tonk, &repository.did())
                .await
                .unwrap();
            let onboarding = crate::onboarding::did(&tonk)
                .await
                .expect("the onboarding account reads")
                .expect("creating a space minted an onboarding account");
            assert_eq!(
                prefix.audience(),
                &onboarding,
                "with no root, the space delegates to the device's onboarding account"
            );
        }

        let (app, state, _lsp) = super::api_router_with_state(test_state_without_account().await);
        let key = put_repo(&app, "signed-out").await;
        let tonk = state.read().await;
        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let prefix = super::repository::space_root_prefix(&tonk, &repository.did())
            .await
            .unwrap();
        let root = super::identity::local_root(&tonk).await.unwrap();
        assert_eq!(
            prefix.audience(),
            &root.root_did,
            "with a bare root, the space delegates to it"
        );
    }

    /// A space created before any root existed is re-issued once one does:
    /// its custodied seed re-delegates to the root and the stored prefix is
    /// replaced, so the account holds the authority going forward.
    #[dialog_common::test]
    async fn it_adopts_profile_spaces_once_a_root_exists() {
        let (app, state, _lsp) = super::api_router_with_state(test_state_without_root().await);
        let key = put_repo(&app, "adopted").await;

        let tonk = state.read().await;
        let root_did = persist_test_root(&tonk).await;

        super::rotation::rotate_from_onboarding(&tonk).await;

        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let prefix = super::repository::space_root_prefix(&tonk, &repository.did())
            .await
            .unwrap();
        assert_eq!(
            prefix.audience(),
            &root_did,
            "adoption re-delegates the space to the root"
        );
    }

    /// An attached account is the whole precondition — a space creates the
    /// moment one exists.
    ///
    /// Through `PUT /api/repository/{label}` rather than `POST /api/spaces`,
    /// because both reach the same gate in `create_repository` and only the
    /// latter goes on to seed the scaffold, which needs a library this
    /// harness serves over no HTTP. `put_repo` asserts the 201 itself.
    #[dialog_common::test]
    async fn it_creates_a_space_once_an_account_is_attached() {
        let (app, _state, _lsp) = super::api_router_with_state(test_state().await);
        let key = put_repo(&app, "account-attached").await;
        assert!(key.starts_with("did:key:"));
    }

    #[dialog_common::test]
    async fn it_presents_space_root_device_and_session_cids_for_proof() {
        use dialog_capability::Subject;

        let state = test_state().await;
        let root = super::identity::local_root(&state).await.unwrap();
        let grant_cid = root.delegation.proof_cids()[0];
        let (app, state, _lsp) = super::api_router_with_state(state);
        let first = put_repo(&app, "First").await;
        let second = put_repo(&app, "Second").await;
        let tonk = state.read().await;

        for key in [first, second] {
            let repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            let proof = tonk
                .profile
                .access()
                .prove(Subject::from(repository.did()))
                .audience(&tonk.operator)
                .perform(&tonk.operator)
                .await
                .unwrap();
            let cids: Vec<_> = proof.proofs.iter().map(|proof| proof.0.to_cid()).collect();
            assert_eq!(cids.len(), 3);
            assert_eq!(cids[1], grant_cid);
            assert_eq!(proof.proofs[0].0.audience(), root.delegation.issuer());
            assert_eq!(proof.proofs[1].0.audience(), &tonk.profile.did());
            assert_eq!(proof.proofs[2].0.issuer(), &tonk.profile.did());
            let stored = super::repository::space_root_prefix(&tonk, &repository.did())
                .await
                .unwrap();
            assert_eq!(stored.proof_cids()[0], cids[0]);
        }
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

    /// `POST /api/sync` is the idle heartbeat's poll target. It must do NO work
    /// of its own — the drain is scheduled by the SW's `on_fetch` seeing the
    /// request, so the route only has to exist and ack. Even with no repository
    /// open it returns `200 {"ok": true}` immediately (it never touches state),
    /// which is exactly why a poll participates in the debounce instead of
    /// forcing a fresh drain per call.
    #[dialog_common::test]
    async fn it_acks_the_idle_sync_poll_without_draining() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);

        let request = Request::builder()
            .uri("/api/sync")
            .method("POST")
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
        let json: serde_json::Value = serde_json::from_slice(&body).expect("response body is JSON");
        assert_eq!(json, serde_json::json!({ "ok": true }));
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

        // Create the repo first so the invite handler can load it. The
        // route refuses a local-only repo, so give it a remote too —
        // this test is only proving the route is reachable.
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        attach_remote(&app, repo, "https://sync.example.test/ucan/").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{}/invite", repo))
                    .method("POST")
                    // The mint derives the link's prefix from the request
                    // origin, which the browser-to-axum conversion stamps on
                    // every real request; a hand-built one has to supply it.
                    .extension(
                        crate::axum::RequestOrigin::parse("https://local.example/invite")
                            .expect("valid origin"),
                    )
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

    /// A freshly created repo plus the invite the `tonk:invite` command
    /// minted for it: the stored `access` chain, the `&remote=` suffix, and
    /// the seed — all read back from the `tonk:invitation` join (durable
    /// authorization + overlay credential), keyed by the repo's subject
    /// DID. The worker mints the keypair, so the test reads the seed back
    /// rather than generating it.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    struct MintedInvite {
        access: String,
        /// The stored `&remote=<url>` suffix.
        remote: String,
        /// The base58 membership seed the worker minted, read back from the
        /// session overlay via the `tonk:invitation` join.
        seed: [u8; 32],
        /// The finished invite URL the handler assembled — what the share
        /// view renders and the user copies. Read back from the overlay, so
        /// tests can assert on the handler's ACTUAL output rather than on a
        /// URL they reassembled from parts themselves.
        link: String,
    }

    /// PUT a fresh repo and return both its routing key and subject DID.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) async fn put_repo_info(app: &Router, label: &str) -> (String, String) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{label}"))
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
        let info: super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        (info.name, info.subject.as_str().to_owned())
    }

    /// Attach a sync remote to `repo`'s `main` branch via `POST /remote` —
    /// the same path the topbar "Enable sync" form drives. Tests that mint
    /// an invite need this first: `run_invite` refuses to mint against a
    /// repo whose `main` has no upstream (see
    /// `repository::tests::it_refuses_to_mint_without_a_remote`).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) async fn attach_remote(app: &Router, repo: &str, endpoint: &str) {
        use super::repository::{
            BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
        };
        use dialog_remote_ucan_s3::UcanAddress;
        use dialog_repository::SiteAddress;

        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(endpoint)))
                    .revocation_url("https://relay.example.test/revocations".parse().unwrap()),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let attach = app
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
        assert_eq!(
            attach.status(),
            StatusCode::OK,
            "remote attach should succeed"
        );
    }

    /// Hand-craft an audience-open invite URL for a synthetic repository
    /// subject. The subject signer doubles as root issuer. Distinct tag
    /// bytes give distinct subjects/ephemerals. Returns the URL plus the
    /// subject's routing key (the repo a join mounts the claimer's
    /// replica under).
    ///
    /// `remote` advertises an access service. Every host used in tests
    /// is unresolvable, so a staged pull against one fails the way a
    /// remote outage does; `None` makes the invite local-only.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) async fn open_invite_url(
        subject_tag: u8,
        ephemeral_tag: u8,
        remote: Option<&str>,
    ) -> (String, String) {
        use dialog_credentials::ed25519::Ed25519Signer;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain};
        use dialog_varsig::Principal as _;
        use tonk_invite::{Invite, InviteAudience};
        use tonk_schema::prelude::DidExt as _;

        let subject_signer = Ed25519Signer::import(&[subject_tag; 32]).await.unwrap();
        let subject = subject_signer.did();
        let key = subject.repo_key().to_owned();
        let ephemeral_seed = [ephemeral_tag; 32];
        let ephemeral = Ed25519Signer::import(&ephemeral_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(subject_signer))
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let invite = Invite::new(
            DelegationChain::new(delegation),
            InviteAudience::Open {
                seed: ephemeral_seed,
            },
            remote.map(|url| url::Url::parse(url).unwrap()),
        )
        .await
        .unwrap()
        .with_revocation_url(
            remote.map(|_| "https://relay.example.test/revocations/".parse().unwrap()),
        );
        (invite.to_url("https://tonk.network/join").unwrap(), key)
    }

    /// Drive the `tonk:invite` command end to end on a fresh, synced repo
    /// and return the minted invite. Attaches a remote before minting —
    /// a local-only repo now refuses to mint at all.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    async fn mint_invite_via_command(app: &Router, label: &str) -> MintedInvite {
        let (repo, subject) = put_repo_info(app, label).await;
        attach_remote(app, &repo, "https://sync.example.test/ucan/").await;
        mint_invite_for(app, &repo, &subject).await
    }

    /// Drive the `tonk:invite` command against an existing `repo` and read
    /// back the resulting `tonk:invitation` keyed by `subject`. Panics if
    /// the handler never produces the join (durable authorization +
    /// overlay credential).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    async fn mint_invite_for(app: &Router, repo: &str, subject: &str) -> MintedInvite {
        // Assert the `tonk:invite` transient via /transact — the path that
        // dispatches commands post-commit. The command carries only a
        // timestamp; the worker mints the keypair and delegation.
        let body = serde_json::json!({
            "claims": [{
                "op": "assert",
                "application": {
                    "parameters": { "time": 1, "marker": "tonk:invite" },
                    "predicate": { "kind": "transient", "concept": {
                        "description": "Mint a repo invite — generates a membership keypair and delegation.",
                        "with": {
                            "time": { "as": "Float", "cardinality": "one", "description": "",
                                "the": "dom.event/time-stamp" },
                            // Per-command marker — distinguishes `tonk:invite`
                            // from other same-shape commands (e.g. pause-sync).
                            "marker": { "as": "Entity", "cardinality": "one", "description": "",
                                "the": "dom.event.current-target.dataset/invite" },
                            "prevent-default": { "cardinality": "one", "description": "",
                                "the": "dom.event.do/prevent-default" }
                        }
                    } }
                }
            }]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/transact"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "transact of tonk:invite should commit",
        );

        // Dispatch runs post-commit; poll the `tonk:invitation` join keyed
        // by the repo subject — `access`/`remote` from the durable
        // authorization, `code` (the seed) and `link` (the assembled URL)
        // from the session overlay.
        for _ in 0..50 {
            let q = serde_json::json!({
                "terms": {
                    "this": subject,
                    "access": { "?": { "name": "access" } },
                    "remote": { "?": { "name": "remote" } },
                    "code": { "?": { "name": "code" } },
                    "link": { "?": { "name": "link" } }
                },
                "predicate": { "with": {
                    "access": { "the": "xyz.tonk.authorization/proof", "as": "Text", "cardinality": "one" },
                    "remote": { "the": "xyz.tonk.authorization/remote", "as": "Text", "cardinality": "one" },
                    "code": { "the": "xyz.tonk.credential/seed", "as": "Text", "cardinality": "one" },
                    "link": { "the": "xyz.tonk.credential/link", "as": "Text", "cardinality": "one" }
                } }
            });
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/repository/{repo}/branch/main/query"))
                        .method("POST")
                        .header("content-type", "application/json")
                        .header("accept", "application/json")
                        .body(Body::from(q.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap();
            let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
            if let Some(row) = rows.first() {
                let access = row["fields"]["access"].as_str().unwrap_or_default();
                assert!(!access.is_empty(), "invitation has empty access");
                let remote = row["fields"]["remote"].as_str().unwrap_or_default();
                let code = row["fields"]["code"].as_str().unwrap_or_default();
                let seed: [u8; 32] = bs58::decode(code)
                    .into_vec()
                    .expect("overlay seed must be valid base58")
                    .as_slice()
                    .try_into()
                    .expect("overlay seed must be 32 bytes");
                let link = row["fields"]["link"].as_str().unwrap_or_default();
                assert!(!link.is_empty(), "invitation has empty link");
                return MintedInvite {
                    access: access.to_owned(),
                    remote: remote.to_owned(),
                    seed,
                    link: link.to_owned(),
                };
            }
            // Yield to the microtask queue so the detached dispatch future
            // makes progress (`tokio::time::sleep` is unsupported on wasm).
            wasm_yield().await;
        }
        panic!("invitation for subject {subject} never appeared after dispatch");
    }

    /// The FAB dispatches enable-sync through the profile branch even though
    /// its result is written to the named space. A standing space subscription
    /// must receive that link when dispatch drains the command's writes;
    /// otherwise the share control has nothing to settle its clipboard promise
    /// with and times out.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_broadcasts_the_invite_minted_by_profile_routed_enable_sync() {
        use futures_util::FutureExt as _;
        use http_body_util::BodyExt as _;

        let state = test_state().await;
        let (app, state, _lsp) = super::api_router_with_state(state);
        let (repo, subject) = put_repo_info(&app, "enable-sync-share-broadcast").await;
        let query = serde_json::json!({
            "predicate": { "with": { "link": {
                "the": "xyz.tonk.credential/link", "as": "Text", "cardinality": "one"
            } } },
            "terms": { "this": subject, "link": { "?": { "name": "link" } } }
        });
        let mut body = open_subscription_with_query(&app, &repo, "main", query).await;
        let snapshot = read_sse_frame(&mut body).await;
        assert_eq!(
            snapshot["conclusions"].as_array().map(Vec::len),
            Some(0),
            "local-only space starts without a credential link",
        );

        let command = serde_json::json!({
            "claims": [{
                "op": "assert",
                "application": {
                    "parameters": {
                        "space": subject,
                        "remote": "https://sync.example.test/ucan/",
                        "revocation": "https://relay.example.test/revocations",
                        "share": "tonk:share",
                        "time": 1,
                        "marker": "tonk:enable-sync"
                    },
                    "predicate": { "kind": "transient", "concept": {
                        "description": "Attach a remote and mint an invite.",
                        "with": {
                            "time": { "the": "dom.event/time-stamp", "as": "Float" },
                            "marker": { "the": "dom.event.current-target.dataset/enable-sync", "as": "Entity" },
                            "space": { "the": "xyz.tonk.enable-sync/space", "as": "Entity" },
                            "remote": { "the": "xyz.tonk.enable-sync/remote", "as": "Text" },
                            "revocation": { "the": "xyz.tonk.enable-sync/revocation-url", "as": "Text" },
                            "share": { "the": "xyz.tonk.enable-sync/share", "as": "Entity" }
                        }
                    } }
                }
            }]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/profile/branch/main/transact")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(command.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // `/transact` detaches command dispatch after committing the transient.
        // Wait until its durable invitation lands, then inspect the already-open
        // stream. Absence after minting completed is the regression.
        for _ in 0..50 {
            if !content_invitations(&state, &repo).await.is_empty() {
                break;
            }
            wasm_yield().await;
        }
        assert_eq!(
            content_invitations(&state, &repo).await.len(),
            1,
            "enable-sync should finish minting the invitation",
        );
        let mut frame = None;
        for _ in 0..10 {
            if let Some(ready) = body.frame().now_or_never().flatten() {
                frame = Some(ready.expect("SSE frame should be readable"));
                break;
            }
            wasm_yield().await;
        }
        let frame = frame.expect("enable-sync mint should broadcast its link");
        let bytes = frame.into_data().expect("data frame");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        let delta: serde_json::Value = serde_json::from_str(
            text.strip_prefix("data: ")
                .and_then(|text| text.strip_suffix("\n\n"))
                .expect("SSE-framed body"),
        )
        .expect("delta is JSON");
        // An ordinary mint on a quiet handle broadcasts an incremental
        // `asserted` delta; a snapshot (`conclusions`) arrives when a poll
        // serves a pending subscriber instead. The FAB accepts both frame
        // kinds, so this does too. (The in-place `refresh_branch` no longer
        // rebinds the engine mid-flow, so no empty replacement snapshot can
        // race in front of the invite delta — that was a CI flake.)
        let rows = delta["conclusions"]
            .as_array()
            .or_else(|| delta["asserted"].as_array())
            .expect("broadcast carries conclusion rows");
        let link = rows[0]["fields"]["link"].as_str().unwrap_or_default();
        assert!(!link.is_empty(), "broadcast carries the minted invite link");
    }

    /// The `tonk:invite` command handler asserts a queryable
    /// `tonk:invitation` join. (`mint_invite_via_command` panics if it
    /// doesn't.)
    #[dialog_common::test]
    async fn it_dispatches_the_invite_command() {
        let state = test_state().await;
        let (app, _state, _lsp) = super::api_router_with_state(state);
        let minted = mint_invite_via_command(&app, "invite-command").await;
        assert!(!minted.access.is_empty());
    }

    /// The minted credential lives in the session overlay only — it must
    /// never reach durable storage. That covers both the seed and the
    /// assembled invite `link`, which carries the seed in its `#` fragment
    /// and so is exactly as secret. Proof: after the overlay is cleared
    /// (which drops only in-memory facts, never touching the branch tree),
    /// both are gone but the durably-committed authorization proof remains.
    #[dialog_common::test]
    async fn it_keeps_the_credential_seed_out_of_storage() {
        let state = test_state().await;
        let (app, app_state, _lsp) = super::api_router_with_state(state);

        let (repo, subject) = put_repo_info(&app, "invite-no-leak").await;
        attach_remote(&app, &repo, "https://sync.example.test/ucan/").await;
        let minted = mint_invite_for(&app, &repo, &subject).await;
        assert!(!minted.access.is_empty(), "authorization must mint");

        // Clear the overlay directly — this drops only the in-memory
        // session facts; durable storage is untouched.
        {
            let tonk = app_state.read().await;
            let session = tonk
                .reactor
                .repository(&repo)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .unwrap();
            session.state.clear_overlay();
        }

        let query = |attr: &str, name: &str| {
            serde_json::json!({
                "terms": { "this": subject, name: { "?": { "name": name } } },
                "predicate": { "with": {
                    name: { "the": attr, "as": "Text", "cardinality": "one" }
                } }
            })
        };
        let read = |q: serde_json::Value| {
            let app = app.clone();
            let repo = repo.clone();
            async move {
                let r = app
                    .oneshot(
                        Request::builder()
                            .uri(format!("/api/repository/{repo}/branch/main/query"))
                            .method("POST")
                            .header("content-type", "application/json")
                            .header("accept", "application/json")
                            .body(Body::from(q.to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
                    .await
                    .unwrap();
                serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).unwrap()
            }
        };

        let proof = read(query("xyz.tonk.authorization/proof", "proof")).await;
        let seed = read(query("xyz.tonk.credential/seed", "seed")).await;
        let link = read(query("xyz.tonk.credential/link", "link")).await;

        assert_eq!(proof.len(), 1, "authorization proof must be durable");
        assert_eq!(
            seed.len(),
            0,
            "credential seed must be overlay-only, never persisted (got {seed:?})",
        );
        assert_eq!(
            link.len(),
            0,
            "invite link carries the seed in its fragment, so it must be \
             overlay-only too, never persisted (got {link:?})",
        );
    }

    /// End-to-end share → join: an invite minted by the `tonk:invite`
    /// command (worker-held keypair; the seed lives in the session overlay
    /// and the URL fragment) must be redeemable through
    /// `POST /api/profile/join`. This is the load-bearing proof that the
    /// command-minted invitation is a valid audience-open invite — the
    /// chain the handler stored as `access`, joined with the overlay seed,
    /// claims as a fresh space.
    #[dialog_common::test]
    async fn it_joins_an_invite_minted_by_the_command() {
        use tonk_invite::{Invite, InviteAudience};

        let state = test_state().await;
        let (app, _state, _lsp) = super::api_router_with_state(state);
        let minted = mint_invite_via_command(&app, "invite-join").await;

        // Assemble the invite URL exactly as the view does: the stored
        // `access` chain plus the `#seed` read back from the overlay.
        let chain_bytes = bs58::decode(&minted.access)
            .into_vec()
            .expect("stored access must be valid base58");
        let chain = dialog_ucan_core::DelegationChain::try_from(chain_bytes.as_slice())
            .expect("stored access must decode to a delegation chain");
        let invite = Invite::new(chain, InviteAudience::Open { seed: minted.seed }, None)
            .await
            .expect("command-minted chain + held seed must form a valid open invite");
        let url = invite
            .to_url(tonk_invite::DEFAULT_BASE_URL)
            .expect("invite must serialize to a URL");

        // Redeem it through the join route. The minter and the redeemer
        // share this test's single profile, so the subject is already
        // mounted (the mint created it) and the outcome is `renewed`
        // rather than `joined` — both are success; the load-bearing
        // assertion is that the command-minted URL *claims* (a 2xx with a
        // replica), proving the chain + seed assemble into a valid,
        // redeemable audience-open invite.
        let (status, body) = post_join(&app, &url).await;
        assert!(
            status.is_success(),
            "joining a command-minted invite should succeed, got {status}: {body}",
        );
        let outcome = body["outcome"].as_str().unwrap_or_default();
        assert!(
            outcome == "joined" || outcome == "renewed",
            "expected a joined/renewed outcome from a command-minted invite, got {outcome:?}: {body}",
        );
        assert!(
            body["repository"]["subject"].is_string(),
            "a successful join must return the claimed repository's subject: {body}",
        );
    }

    /// The URL the handler actually minted — the one the share view renders
    /// and the user copies — is itself redeemable.
    ///
    /// The sibling test above rebuilds the URL from the stored `access` +
    /// `seed`, so it would still pass if `link` were malformed or empty.
    /// This one joins through `link` verbatim, closing that gap: it is the
    /// only test that fails if the mint hands the user a broken link.
    ///
    /// It also pins the shortening fallback. The harness's worker scope
    /// reports no `location.origin` and there is no shortcut service, so the
    /// mint takes the no-origin path and shortening never happens — and that
    /// fallback must still yield a *working* invite, not a degraded one. The
    /// origin branch is covered by `it_builds_the_invite_url_on_the_worker_origin`
    /// in `repository.rs`, which drives the URL builder directly.
    #[dialog_common::test]
    async fn it_joins_through_the_minted_link() {
        let state = test_state().await;
        let (app, _state, _lsp) = super::api_router_with_state(state);
        let minted = mint_invite_via_command(&app, "invite-link-join").await;

        // The seed rides in the fragment, never the query.
        let seed = bs58::encode(minted.seed).into_string();
        assert!(
            minted.link.ends_with(&format!("#{seed}")),
            "the minted link must carry the seed as its fragment: {}",
            minted.link,
        );
        assert!(
            minted.link.contains(&format!("access={}", minted.access)),
            "the minted link must carry the delegation chain: {}",
            minted.link,
        );

        // Redeem the handler's own URL, unmodified.
        let (status, body) = post_join(&app, &minted.link).await;
        assert!(
            status.is_success(),
            "the minted link must redeem, got {status}: {body}",
        );
        let outcome = body["outcome"].as_str().unwrap_or_default();
        assert!(
            outcome == "joined" || outcome == "renewed",
            "expected a joined/renewed outcome from the minted link, got {outcome:?}: {body}",
        );
    }

    /// A command-minted invite for a *synced* repo must embed the sync
    /// endpoint as a `&remote=` query parameter, so a recipient on another
    /// device knows where to pull the shared content from. (A local-only
    /// repo has nothing to embed — it refuses to mint at all, covered by
    /// `repository::tests::it_refuses_to_mint_without_a_remote` and
    /// `create_invite::tests::it_refuses_a_repository_with_no_upstream`.)
    #[dialog_common::test]
    async fn it_embeds_the_remote_in_a_command_minted_invite() {
        let state = test_state().await;
        let (app, _state, _lsp) = super::api_router_with_state(state);

        // Create the repo, then attach a sync remote via the `/remote`
        // route (the same path the topbar "Enable sync" form drives).
        let (repo, subject) = put_repo_info(&app, "invite-remote").await;
        let endpoint = "https://sync.example.test/ucan/";
        attach_remote(&app, &repo, endpoint).await;

        // Mint the invite through the command. `mint_invite_via_command`
        // creates its *own* fresh repo, so mint against THIS repo directly:
        // assert the transient, read back the invitation keyed by subject.
        let minted = mint_invite_for(&app, &repo, &subject).await;

        assert!(
            minted.remote.contains("&remote="),
            "a synced repo's invitation must carry a &remote= suffix, got {:?}",
            minted.remote,
        );
        assert!(
            minted.remote.contains("sync.example.test"),
            "the &remote= suffix must point at the attached endpoint, got {:?}",
            minted.remote,
        );
    }

    /// Yield to the JS event loop (a `setTimeout(0)`-backed delay) so a
    /// detached `spawn_local` future advances between polls. `tokio`'s
    /// timer is unsupported on `wasm32`.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    async fn wasm_yield() {
        use wasm_bindgen::JsCast;
        let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, _| {
            let scope: web_sys::ServiceWorkerGlobalScope = js_sys::global().unchecked_into();
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
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
            .issuer(dialog_credentials::Signer::from(subject_signer))
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
    async fn it_rejects_manual_sync_without_an_upstream() {
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

        // Manual sync is an attempted operation, not the background sweep:
        // without an upstream it must report an operational failure rather
        // than claim a successful reconciliation. The background coordinator
        // deliberately filters such branches before calling this route.
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

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let failure: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(failure["error"]["code"], "SYNC_UNAVAILABLE");
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
    /// same pattern the tonk sync tests use — so the status route's
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

    /// A repo whose only upstreamed branch cannot reach its remote must
    /// resolve as `Err`, naming the repo — that is what re-marks it dirty
    /// in `drain_sync` so the next heartbeat retries it. A silent `Ok`
    /// here would drop the repo out of the work queue on first failure.
    ///
    /// This is a real, deterministic failure — a loopback connection
    /// refused, no external network or DNS involved — not a faked one.
    #[dialog_common::test]
    async fn it_reports_a_reconcile_failure_for_an_unreachable_upstream() {
        use super::repository::{
            BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
        };
        use dialog_remote_ucan_s3::UcanAddress;
        use dialog_repository::SiteAddress;

        let state = test_state().await;
        let (app, app_state, _lsp) = super::api_router_with_state(state);

        let (repo, _subject) = put_repo_info(&app, "sync-repo-unreachable").await;

        // A real remote address on loopback with nothing listening: the
        // connection is refused immediately (no external network, no
        // DNS lookup, nothing to time out), so the sweep's pull fails
        // deterministically.
        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(
                    "http://127.0.0.1:1/ucan/",
                ))),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let attach = app
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
        assert_eq!(
            attach.status(),
            StatusCode::OK,
            "remote attach should succeed (it never touches the network)"
        );

        let message = super::sync_repository(&app_state, &repo)
            .await
            .expect_err("an unreachable upstream must not resolve as a clean sweep");
        assert!(
            message.contains(&repo),
            "the error should name the repo that failed to reconcile: {message}"
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

        let body = r#"attribute!: &person-name
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
"#;

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

        let body = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name
"#;

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
    /// attribute by `db.meta/name`, reconstructs its
    /// descriptor, and lets the concept hash correctly.
    #[dialog_common::test]
    async fn it_resolves_bookmarks_across_transactions() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-transact-cross";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // First transaction: define `person-name` only.
        let first = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name
"#;
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
        let second = r#"attribute!: &person-age
  the:         io.gozala.person/age
  as:          unsigned-integer
  cardinality: one
  description: The person's age

concept!: &person
  description: A person
  with:
    name: person-name
    age:  person-age
"#;
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
        let setup = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &alice
  name: "Alice"
"#;
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
        let setup = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &alice
  name: "Alice"
"#;
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
        let setup = r#"attribute!: &person-name
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
  name: "Alice"
  age:  28
"#;
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
        let update_via_query = r#"person:
  this: ?alice
  name: "Alice"
person!:
  this: ?alice
  age:  29
"#;
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
    /// `db.meta/name`). After the rebind, the old entity
    /// still
    /// holds its facts but is no longer addressable by `.alice`.
    #[dialog_common::test]
    async fn it_rebinds_bookmark_on_body_change() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-rebind-bookmark";

        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        let setup = r#"attribute!: &person-name
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
  name: "Alice"
  age:  28
"#;
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let alice_v1 = setup_resp.commits.entities.get("alice").cloned().unwrap();

        // Same anchor name, different body → new entity.
        let rebind = r#"person!: &alice
  name: "Alice"
  age:  29
"#;
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
        let follow_up = r#"person:
  this: ?p
  name: ?n
"#;
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

        let setup = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name
"#;
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

        let setup = r#"attribute!: &person-name
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
"#;
        let _ = post_yaml(&app, repo, setup).await;

        // Single head introduces `?carol` and writes both fields.
        // (Two heads with the same `person!:` body containing
        // `this: ?carol` would collapse to one expression at the
        // YAML mapping-key level — that's a parser-side detail,
        // not a semantic limit of variable scope.)
        let doc = r#"person!:
  this: ?carol
  name: "Carol"
  age:  31
"#;
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

        let setup = r#"attribute!: &tagged-name
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
  name: "Dave"
  tag:  "engineer"
"#;
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

        let setup = r#"attribute!: &person-name
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
  name: "Erin"
  age:  41
"#;
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

        let setup = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: The person's name

concept!: &person
  with:
    name: person-name

person!: &frank
  name: "Frank"
"#;
        let _ = post_yaml(&app, repo, setup).await;

        let retract = r#"person:
  this: ?frank
  name: "Frank"
person!:
  this: ?frank
  ..: _
"#;
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
        let setup = r#"attribute!: &person-name
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
  name: "Gina"
"#;
        let body_bytes = post_yaml(&app, repo, setup).await;
        let setup_resp: super::EvaluateResponse = serde_json::from_slice(&body_bytes).unwrap();
        let gina_uri = setup_resp.commits.entities.get("gina").cloned().unwrap();

        // Add an employee fact on Gina's entity.
        let add_emp = format!("employee!:\n  this: {gina_uri}\n  eid: \"E-007\"\n");
        let _ = post_yaml(&app, repo, &add_emp).await;

        // Joined query.
        let join = r#"person:
  this: ?p
  name: ?n
employee:
  this: ?p
  eid: ?e
"#;
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
            r#"attribute!: &{name}
  the:         {the}
  as:          text
  cardinality: one
  description: A test attribute
"#
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
        assert!(kind == "index" || kind == "segment", "kind set: {row}");
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

    /// Run a one-shot formula `/query` with explicit `terms`.
    async fn post_formula_query_with(
        app: &Router,
        repo: &str,
        branch: &str,
        formula: &str,
        terms: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let wire = serde_json::json!({ "predicate": formula, "terms": terms });
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

    /// `tree/child` walks one level down: given the root node's hash,
    /// it returns one self-contained row per child — each carrying the
    /// child's own hash, sibling position, and node fields. (If the
    /// root is itself a leaf, there are no children and zero rows; the
    /// query still succeeds.)
    #[dialog_common::test]
    async fn it_resolves_tree_children_of_the_root() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-formula-child";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        // Find the root and its kind.
        let (_, root_body) = post_formula_query(&app, repo, "main", "tree/node").await;
        let root = &root_body.as_array().expect("array")[0];
        let root_hash = root["this"].as_str().expect("root hash").to_string();
        let root_is_index = root["fields"]["kind"] == "index";

        let (status, body) = post_formula_query_with(
            &app,
            repo,
            "main",
            "tree/child",
            serde_json::json!({ "hash": root_hash }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tree/child OK: {body}");
        let rows = body.as_array().expect("array of children");

        if root_is_index {
            assert!(!rows.is_empty(), "an index root has children: {body}");
            for (i, row) in rows.iter().enumerate() {
                assert!(
                    row["fields"]["child"]
                        .as_str()
                        .is_some_and(|s| s.starts_with('#')),
                    "child hash present: {row}"
                );
                assert_eq!(row["this"], row["fields"]["child"], "this == child: {row}");
                assert_eq!(row["fields"]["at"], i as i64, "sibling position: {row}");
                let kind = row["fields"]["kind"].as_str().unwrap_or("");
                assert!(
                    kind == "index" || kind == "segment",
                    "child kind set: {row}"
                );
            }
        } else {
            assert!(rows.is_empty(), "a segment root has no children: {body}");
        }
    }

    /// A full walk: root → (descend branches to) a leaf → its entries
    /// → decode one entry's key into components. Exercises tree/node,
    /// tree/child, tree/entry, and tree/key chained by feeding each
    /// row's hash into the next query.
    #[dialog_common::test]
    async fn it_walks_to_a_leaf_and_decodes_an_entry_key() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-formula-walk";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        // Descend from the root to the first segment.
        let (_, root_body) = post_formula_query(&app, repo, "main", "tree/node").await;
        let mut hash = root_body.as_array().unwrap()[0]["this"]
            .as_str()
            .unwrap()
            .to_string();
        let mut kind = root_body.as_array().unwrap()[0]["fields"]["kind"]
            .as_str()
            .unwrap()
            .to_string();
        let mut guard = 0;
        while kind == "index" && guard < 32 {
            guard += 1;
            let (_, kids) = post_formula_query_with(
                &app,
                repo,
                "main",
                "tree/child",
                serde_json::json!({ "hash": hash }),
            )
            .await;
            let first = &kids.as_array().expect("children")[0];
            hash = first["fields"]["child"].as_str().unwrap().to_string();
            kind = first["fields"]["kind"].as_str().unwrap().to_string();
        }
        assert_eq!(kind, "segment", "descended to a segment");

        // Read the segment's entries.
        let (status, entries) = post_formula_query_with(
            &app,
            repo,
            "main",
            "tree/entry",
            serde_json::json!({ "hash": hash }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tree/entry OK: {entries}");
        let entries = entries.as_array().expect("entries");
        assert!(!entries.is_empty(), "segment has entries: {entries:?}");
        let entry = &entries[0];
        assert!(
            entry["fields"]["key"]
                .as_str()
                .is_some_and(|s| s.starts_with("0x")),
            "entry carries a hex key: {entry}"
        );
        assert!(
            entry["fields"]["attribute"].as_str().is_some(),
            "entry carries its decoded datum: {entry}"
        );

        // Decode that entry's key into components.
        let entry_key = entry["fields"]["key"].as_str().unwrap().to_string();
        let (status, decoded) = post_formula_query_with(
            &app,
            repo,
            "main",
            "tree/key",
            serde_json::json!({ "key": entry_key }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tree/key OK: {decoded}");
        let row = &decoded.as_array().expect("one row")[0];
        let tag = row["fields"]["tag"].as_str().unwrap_or("");
        assert!(
            matches!(tag, "entity" | "attribute" | "value"),
            "key tag named: {row}"
        );
        // The M3 key format is variable-length, so components come back
        // as a decoded `parts` list rather than the fixed-offset
        // `entity` / `value-type` fields the old layout could name.
        let parts = row["fields"]["parts"]
            .as_array()
            .unwrap_or_else(|| panic!("key parts present: {row}"));
        let kinds: Vec<&str> = parts
            .iter()
            .filter_map(|part| part["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"entity"), "entity component present: {row}");
        assert!(
            kinds.contains(&"vtype"),
            "value-type component present: {row}"
        );
    }

    /// Open an SSE subscription and return the open body so the
    /// caller can read frames as they arrive.
    async fn open_subscription(app: &Router, repo: &str, branch: &str) -> Body {
        open_subscription_with_query(app, repo, branch, named_concept_wire_query()).await
    }

    /// Open an SSE subscription for an explicit inline query.
    async fn open_subscription_with_query(
        app: &Router,
        repo: &str,
        branch: &str,
        query: serde_json::Value,
    ) -> Body {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/{branch}/query"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .header("accept", "text/event-stream")
                    .body(Body::from(query.to_string()))
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

    /// Open an SSE subscription, read the first event (a
    /// `{kind:"snapshot", conclusions:[…]}` frame), and return its
    /// `conclusions` array — the same bare-array shape the one-shot
    /// `/query` route returns, so the two are directly comparable.
    /// Drops the body afterwards.
    async fn subscribe_first_event(app: &Router, repo: &str, branch: &str) -> serde_json::Value {
        let mut body = open_subscription(app, repo, branch).await;
        let frame = read_sse_frame(&mut body).await;
        assert_eq!(
            frame.get("kind").and_then(|k| k.as_str()),
            Some("snapshot"),
            "first SSE frame is a snapshot: {frame}"
        );
        frame
            .get("conclusions")
            .cloned()
            .expect("snapshot frame carries a conclusions array")
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

    /// The reactor's one-shot [`query`](crate::reactor::QueryEffect)
    /// effect — what the non-streaming `/query` arm and headless
    /// callers (e.g. `tonk render`) use — returns the same
    /// projected conclusions the HTTP route does, without opening a
    /// subscription.
    #[dialog_common::test]
    async fn it_reads_one_shot_via_the_reactor_query_effect() {
        use dialog_query::{ConceptQuery, Query as ConceptPattern};
        use std::sync::Arc;
        use tokio::sync::RwLock;
        use tonk_schema::meta::Name;

        let tonk = test_state().await;
        let app_state: crate::router::AppState = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = crate::api_router_from_state(app_state.clone());

        let repo = "test-reactor-query-effect";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();
        seed_named_entity(&app, repo).await;

        // Read directly through the reactor effect.
        let query = ConceptQuery::from(ConceptPattern::<Name>::default());
        let conclusions = {
            let guard = app_state.read().await;
            guard
                .reactor
                .repository(repo)
                .branch("main")
                .query(query)
                .perform(&guard.operator)
                .await
                .expect("one-shot query")
        };
        assert!(
            !conclusions.is_empty(),
            "expected at least one named entity after seeding"
        );

        // The effect agrees with the HTTP one-shot route.
        let via_route = post_query(&app, repo, "main").await;
        let via_effect = serde_json::to_value(&conclusions).expect("conclusions serialize");
        assert_eq!(
            via_effect, via_route,
            "reactor query effect matches the one-shot route"
        );
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

        // The first frame is a full snapshot of the current result set.
        let snapshot = read_sse_frame(&mut body).await;
        assert_eq!(
            snapshot.get("kind").and_then(|k| k.as_str()),
            Some("snapshot"),
            "first frame is a snapshot: {snapshot}"
        );
        let before = snapshot
            .get("conclusions")
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        // Commit a second named attribute — the result set grows by one row.
        seed_named_attribute(&app, repo, "person-age", "xyz.tonk.person/age").await;

        // The commit broadcasts an incremental delta: the new row is asserted,
        // nothing retracted, so the result set is strictly larger than before.
        let delta = read_sse_frame(&mut body).await;
        assert_eq!(
            delta.get("kind").and_then(|k| k.as_str()),
            Some("delta"),
            "post-commit frame is a delta: {delta}"
        );
        let asserted = delta
            .get("asserted")
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let retracted = delta
            .get("retracted")
            .and_then(|r| r.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert!(
            asserted > 0,
            "commit must broadcast the new row as asserted: {delta}"
        );
        assert!(
            before + asserted > before + retracted,
            "post-commit result set grows (asserted {asserted} > retracted {retracted})"
        );
    }

    /// A committing `/evaluate` that SUPERSEDES a cardinality-one field
    /// on an entity with an open SSE subscription must deliver a `delta`
    /// frame carrying the NEW value — the "counter" bug shape: one row
    /// whose field moved, not a new row.
    ///
    /// Unlike `it_broadcasts_changed_snapshot_after_commit`, this does
    /// NOT grow the result set. Declaring an attribute anchor publishes
    /// a `meta::Name` (`id:<anchor>` carries a cardinality-one
    /// `db.name/referent` pointing at the attribute entity).
    /// Re-declaring the SAME anchor with a different `the:` re-points
    /// that referent: the `id:<anchor>` row keeps its identity but its
    /// `entity` field changes. The subscription must observe that
    /// supersession as a `delta` frame whose `asserted` row carries the
    /// new target and whose `retracted` row carries the old one.
    ///
    /// NOTE on discriminating power: on the current tree this test
    /// passes with OR without the `run_scheduled_polls` drain in
    /// `router/evaluate.rs`. The `/evaluate` handler also polls inline
    /// via `session.poll()` after its commit, and that inline poll
    /// already re-evaluates every subscription on the committed branch
    /// and fans out the delta. `run_scheduled_polls` is a no-op for a
    /// plain `/evaluate`: the dialog `txn.commit()` schedules no reactor
    /// poll, so the pending-poll set is empty. This test therefore
    /// locks in the correct superseding-delta behavior (and guards
    /// against a regression that would drop the inline poll), but it
    /// does NOT by itself prove the `run_scheduled_polls` fix — see the
    /// handoff note.
    #[dialog_common::test]
    async fn it_broadcasts_superseded_field_after_commit() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-supersede-broadcast";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        // Publish `id:counter -> <attribute entity for xyz.tonk/a>`.
        seed_named_attribute(&app, repo, "counter", "xyz.tonk/a").await;

        let mut body = open_subscription(&app, repo, "main").await;

        // The first frame is a `snapshot` carrying the pre-supersession
        // target for `id:counter`. Keep it so we can prove the
        // post-commit frame moved the field.
        let snapshot_before = read_sse_frame(&mut body).await;
        assert_eq!(
            snapshot_before.get("kind").and_then(|k| k.as_str()),
            Some("snapshot"),
            "first frame is a snapshot: {snapshot_before}"
        );
        let target_before = counter_entity_in(&snapshot_before, "conclusions");

        // Re-declare the SAME anchor pointing at a fresh attribute
        // entity (`the: xyz.tonk/b`). This supersedes `id:counter`'s
        // cardinality-one referent — the row's identity is unchanged,
        // only its `entity` field moves. The result set does NOT grow.
        seed_named_attribute(&app, repo, "counter", "xyz.tonk/b").await;

        // (1) Propagation: a frame must ARRIVE after the superseding
        // commit. If neither the inline `session.poll()` nor the
        // scheduled-poll drain re-evaluated the subscription, no delta
        // reaches the subscriber and this read hangs until the harness
        // timeout.
        let delta_after = read_sse_frame(&mut body).await;
        assert_eq!(
            delta_after.get("kind").and_then(|k| k.as_str()),
            Some("delta"),
            "post-supersession frame is an incremental delta: {delta_after}"
        );

        // (2) Correctness: the delta's `asserted` row for `id:counter`
        // must carry the NEW target, not the stale one that was current
        // when the subscription opened.
        let target_after = counter_entity_in(&delta_after, "asserted");
        assert_ne!(
            target_before, target_after,
            "superseding commit must broadcast the new `entity`, not the stale one; \
             before={target_before:?} after={target_after:?}"
        );
    }

    /// Find the `id:counter` name row inside an SSE frame's `key` array
    /// (`conclusions` for a snapshot, `asserted` for a delta) and
    /// return its projected `entity` target — the referent the name
    /// currently points at. Panics with the offending frame if the row
    /// or field is missing so a shape regression is loud.
    fn counter_entity_in(frame: &serde_json::Value, key: &str) -> String {
        let rows = frame
            .get(key)
            .and_then(|r| r.as_array())
            .unwrap_or_else(|| panic!("frame missing `{key}` array: {frame}"));
        for row in rows {
            let fields = row
                .get("fields")
                .and_then(|f| f.as_object())
                .unwrap_or_else(|| panic!("row missing fields object: {row}"));
            if fields.get("this").and_then(|v| v.as_str()) == Some("id:counter") {
                return fields
                    .get("entity")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("id:counter row missing `entity`: {row}"))
                    .to_string();
            }
        }
        panic!("no id:counter row in frame `{key}`: {frame}");
    }

    /// TWO consecutive supersessions over one open subscription. This
    /// pins down whether the reactor tracks the state IT last emitted:
    /// the second delta's `retracted` must carry the value the FIRST
    /// delta asserted (b), not the original snapshot value (a). If the
    /// reactor's per-subscription base drifts (retract still names the
    /// original), a consumer applying deltas by value-identity can't
    /// match the retract against its own retained row and mis-renders.
    #[dialog_common::test]
    async fn it_tracks_emitted_state_across_consecutive_supersessions() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let repo = "test-reactor-consecutive-supersede";
        let key = put_repo(&app, repo).await;
        let repo = key.as_str();

        seed_named_attribute(&app, repo, "counter", "xyz.tonk/a").await;

        let mut body = open_subscription(&app, repo, "main").await;
        let snapshot = read_sse_frame(&mut body).await;
        let target_a = counter_entity_in(&snapshot, "conclusions");

        // First supersession: a -> b.
        seed_named_attribute(&app, repo, "counter", "xyz.tonk/b").await;
        let delta1 = read_sse_frame(&mut body).await;
        let asserted_b = counter_entity_in(&delta1, "asserted");
        let retracted_1 = counter_entity_in(&delta1, "retracted");
        assert_eq!(
            retracted_1, target_a,
            "delta1 must retract the original (a); got retracted={retracted_1} target_a={target_a}"
        );

        // Second supersession: b -> c. The reactor's base for THIS delta
        // must be `b` (what delta1 asserted), so delta2 retracts `b`.
        seed_named_attribute(&app, repo, "counter", "xyz.tonk/c").await;
        let delta2 = read_sse_frame(&mut body).await;
        let asserted_c = counter_entity_in(&delta2, "asserted");
        let retracted_2 = counter_entity_in(&delta2, "retracted");

        assert_ne!(asserted_b, asserted_c, "delta2 asserts a new target (c)");
        assert_eq!(
            retracted_2, asserted_b,
            "delta2 must retract what delta1 ASSERTED (b), proving the reactor \
             tracks its own emitted state; got retracted={retracted_2} asserted_b={asserted_b}"
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

    /// `/query` against a missing repository answers the empty set, the
    /// same answer a branch that exists and matches nothing gives.
    /// Absence is not a transport failure: a 404 became `offline` on the
    /// page and retried forever.
    #[dialog_common::test]
    async fn it_answers_a_query_against_an_unknown_repo_with_the_empty_set() {
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
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let answer: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            answer,
            serde_json::json!([]),
            "a repo that is not here holds nothing"
        );
    }

    /// `Reactor::shutdown` must drop every active subscriber's
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

        let body = r#"attribute!: &person-name
  the:         io.gozala.person/name
  as:          text
  cardinality: one
  description: "name"
"#;

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
        let seed = r#"attribute!: &person-name
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
  description: "name"

attribute!: &person-age
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one
  description: "age"

concept!: &person
  description: "a person"
  with:
    name: person-name
    age:  person-age
"#;
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

        let assertion = r#"person!:
  name: "Bob"
  age: 2
"#;
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
        let seed = r#"attribute!: &person-name
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
  description: "name"

attribute!: &person-age
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one
  description: "age"

concept!: &person
  description: "a person"
  with:
    name: person-name
    age:  person-age

person!:
  name: "Alice"
  age:  29
"#;
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
        let query = r#"person:
  name: _
  age:  _
"#;
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

#[cfg(test)]
mod handshake_tests {
    use super::{is_mutating, stale_build};

    /// The whole point: a page and a worker from different builds must
    /// be caught, because `skipWaiting` + `claim` swap the worker
    /// underneath a running page and the HTTP surface is not versioned.
    #[dialog_common::test]
    fn it_refuses_a_page_from_another_build() {
        assert_eq!(
            stale_build(Some("worker-build"), Some("page-build")),
            Some(("worker-build".to_owned(), "page-build".to_owned())),
        );
    }

    /// The matched case is the overwhelmingly common one and must be
    /// completely transparent.
    #[dialog_common::test]
    fn it_passes_a_matching_build_through() {
        assert_eq!(stale_build(Some("same"), Some("same")), None);
    }

    /// Reads must survive a build mismatch. Observed in a browser: a
    /// `POST …/query` — a SUBSCRIPTION, despite the verb — came back
    /// `409` during an ordinary worker swap, so the page lost its live
    /// updates and had no way to notice. A stale page reading is
    /// harmless; a stale page with dead subscriptions is frozen.
    #[dialog_common::test]
    fn it_refuses_only_writes() {
        use axum::extract::Request;

        let read = Request::builder()
            .uri("/api/profile/branch/main/query")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(
            !is_mutating(&read),
            "a query/subscribe must pass through even from another build"
        );

        for write in [
            "/api/profile/branch/main/transact",
            "/api/repository/x/branch/main/transact",
            "/api/profile/branch/main/evaluate",
        ] {
            let request = Request::builder()
                .uri(write)
                .body(axum::body::Body::empty())
                .unwrap();
            assert!(
                is_mutating(&request),
                "{write} changes state and must be gated"
            );
        }
    }

    /// A request that cannot be classified must never be blocked.
    /// Blocking on a missing header would break every context that
    /// does not send one — a sealed guest, an older page still in a
    /// tab — turning a diagnostic into an outage.
    #[dialog_common::test]
    fn it_never_blocks_what_it_cannot_classify() {
        assert_eq!(
            stale_build(Some("worker-build"), None),
            None,
            "a page that sends no build is not therefore stale"
        );
        assert_eq!(
            stale_build(None, Some("page-build")),
            None,
            "an unstamped worker has no identity to compare against"
        );
        assert_eq!(stale_build(None, None), None);
    }
}
