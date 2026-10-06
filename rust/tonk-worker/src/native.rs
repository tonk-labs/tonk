//! Running the worker as a native process instead of a service worker.
//!
//! In the browser, `TonkServiceWorker` owns the worker state, routes the
//! page's fetches into the [`axum::Router`], and runs the boot chores and
//! the sync loop on the service-worker event loop. A native host (the
//! desktop shell) does the same job from a Tokio runtime: it opens the
//! profile from a filesystem directory, serves the router over HTTP, and
//! drives sync on a timer. [`NativeWorker`] is the part of that which
//! belongs to the worker rather than to the host.
//!
//! Space storage resolves against `Directory::Current` (see
//! `device::space_location`). In the browser that is an IndexedDB
//! namespace; natively it is the process's working directory, so a host
//! must set the working directory to its data directory before calling
//! [`NativeWorker::open`].

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use dialog_effects::storage::Directory;
use tonk_common::log;

use crate::device::{REGISTRY_PROFILE, Registry};
use crate::router::{AppState, LspHub, api_router_with_state};
use crate::worker::boot_state;
use crate::{TonkWorkerError, drain_sync};

/// How often the native sync loop considers a drain. `drain_sync`
/// coalesces overlapping calls and skips paused spaces, so this is a
/// latency bound for seeing another device's change, not a cost.
const SYNC_INTERVAL: Duration = Duration::from_secs(5);

/// The access-service origin a native host was configured with.
static SERVICE_ORIGIN: OnceLock<String> = OnceLock::new();

/// Record the origin account and access services are reached at.
///
/// A service worker answers "where is the service" with its own origin.
/// A native host serves from loopback, which is no service at all, so the
/// answer has to come from host configuration. Without it the worker
/// treats the service as unknown and account features refuse visibly.
/// Only the first call takes effect.
pub fn set_service_origin(origin: impl Into<String>) {
    let _ = SERVICE_ORIGIN.set(origin.into().trim_end_matches('/').to_owned());
}

/// The origin set by [`set_service_origin`], if any.
pub(crate) fn service_origin() -> Option<String> {
    SERVICE_ORIGIN.get().cloned()
}

/// Loads a location in the host's page: the href, and whether it replaces
/// the current history entry.
type Navigator = dyn Fn(&str, bool) + Send + Sync;

/// The navigation a native host with a page was configured with.
static NAVIGATOR: OnceLock<Box<Navigator>> = OnceLock::new();

/// Record how the host loads a location in its page.
///
/// A service worker redirects the page that asked by posting it a
/// `navigate` message. A native host has no such channel, so a command
/// whose effect is a page load (opening a new space, sending the person
/// to approve a sign-in) reaches the page through this instead. `href`
/// is either a path on the page's own origin or an absolute address
/// elsewhere, which a desktop host opens in the system browser. Without
/// it the target is only logged. Only the first call takes effect.
pub fn set_navigator(navigate: impl Fn(&str, bool) + Send + Sync + 'static) {
    let _ = NAVIGATOR.set(Box::new(navigate));
}

/// Load `href` through the host's navigator. Answers whether one was set.
pub(crate) fn navigate(href: &str, replace: bool) -> bool {
    match NAVIGATOR.get() {
        Some(navigate) => {
            navigate(href, replace);
            true
        }
        None => false,
    }
}

/// A worker opened natively: the router a host serves, and the state
/// behind it.
pub struct NativeWorker {
    /// The same router the service worker dispatches into.
    pub router: Router,
    /// Shared worker state, for host-side work outside the request path.
    pub state: AppState,
    /// The language-server hub, so a host can release its streams on
    /// shutdown.
    pub lsp: Arc<LspHub>,
}

impl NativeWorker {
    /// Open the active profile kept in `directory` and build the worker.
    ///
    /// `directory` holds the registry profile, its system key, and every
    /// profile it names. Pass a directory of the host's own: the `tonk`
    /// CLI keeps a profile with the same name in `Directory::Profile`
    /// with a different space layout, and the two must not share it.
    ///
    /// Must be called inside a Tokio runtime: the boot chores and the
    /// sync loop are spawned onto it.
    pub async fn open(directory: Directory) -> Result<Self, TonkWorkerError> {
        let registry = Registry {
            profile: REGISTRY_PROFILE.to_owned(),
            directory,
        };
        let storage = registry.storage().await?;
        let (profile_name, profile) = registry.open_active(&storage).await?;
        log!("Profile DID: {}", profile.did());

        let state = boot_state(storage, profile_name, profile, registry).await?;
        let (router, state, lsp) = api_router_with_state(state);

        spawn_boot_chores(state.clone());
        spawn_sync_loop(state.clone());

        Ok(Self { router, state, lsp })
    }
}

/// The chores the service worker runs once it boots. Each one no-ops
/// when the profile turns out to be unlinked, so they are safe on a
/// profile that has never seen an account.
fn spawn_boot_chores(state: AppState) {
    crate::detach(async move {
        let tonk = state.read().await;
        crate::router::profiles::upsert_active_entry(&tonk, None).await;
        crate::router::account_state::ensure_account_state(&tonk).await;
        crate::router::customer::drain_pending(&tonk).await;
        crate::router::rotation::rotate_from_onboarding(&tonk).await;
    });
}

/// Pull and push on a timer while a page is watching or local work is
/// waiting to go up. The service worker's loop also gates on page
/// visibility and `navigator.onLine`; a native process has neither, and
/// a failed drain already stamps the space offline.
fn spawn_sync_loop(state: AppState) {
    crate::detach(async move {
        loop {
            tokio::time::sleep(SYNC_INTERVAL).await;
            if wants_sync(&state).await {
                drain_sync(&state).await;
            }
        }
    });
}

/// Whether any cached branch holds a live subscriber or any space holds
/// un-pushed commits.
async fn wants_sync(state: &AppState) -> bool {
    let tonk = state.read().await;
    if tonk.sync_queue.dirty_count() > 0 {
        return true;
    }
    let repos = tonk.reactor.repos().read();
    repos.values().any(|repo| {
        repo.branches()
            .read()
            .values()
            .any(|branch| !branch.subscriptions().lock().is_empty())
    })
}
