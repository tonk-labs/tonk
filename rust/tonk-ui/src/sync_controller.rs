//! Background sync controller.
//!
//! Drives automatic reconciliation of the active repository's
//! upstream branches so a local write reaches the remote — and
//! remote changes reach the tab — without anyone clicking Pull or
//! Push. The manual buttons in [`crate::components::space`] stay as
//! they are; this sits on top of the same `/sync` routes.
//!
//! Triggers, all funneling into one re-entrancy-guarded sweep:
//!
//! - a steady [`TICK_INTERVAL_MS`] interval;
//! - the window coming back `online`;
//! - the tab becoming visible again;
//! - a local commit, debounced by [`COMMIT_DEBOUNCE_MS`] so a burst
//!   of edits collapses into a single sync.
//!
//! On by default for every repository; an explicit per-repository
//! `off` preference pauses it (see [`is_enabled`] / [`set_enabled`]).
//! Pausing stops pull/push — but the background triggers still fetch
//! the upstream head read-only on each tick, so the sync-state badges
//! keep showing where local sits relative to remote. A frozen badge
//! would make the pause indicator useless.

use leptos::ev;
use leptos::logging::log;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_use::{use_debounce_fn, use_event_listener, use_interval_fn};
use wasm_bindgen::prelude::*;

use crate::api;

/// How often the controller sweeps the active repository's branches.
/// One knob, tuned in one place.
const TICK_INTERVAL_MS: u64 = 20_000;

/// Debounce window after a local commit before syncing — coalesces
/// a burst of writes into one sweep.
const COMMIT_DEBOUNCE_MS: f64 = 1_000.0;

/// DOM event a successful local commit dispatches on `window`. The
/// controller listens for it to sync shortly after a write.
pub const COMMITTED_EVENT: &str = "tonk:committed";

/// DOM event the controller dispatches on `window` to ask the active
/// repository's branch rows to re-read their read-only sync status.
/// Fired on *every* sweep — after a successful `Sync` sweep as well as
/// on the paused tick — so the sync-state badges keep tracking remote
/// drift whether or not anything is being pulled or pushed, and a
/// consumer that reads status off this event (rather than the SSE
/// `watch` layer) stays current. The active repository name rides in
/// the event's `detail` as a plain string so a consumer can ignore
/// events for other repositories.
pub const STATUS_REFRESH_EVENT: &str = "tonk:status-refresh";

/// Per-repository `localStorage` key holding the auto-sync pause
/// preference. Absent means on (the default).
fn pref_key(repo: &str) -> String {
    format!("tonk:auto-sync:{repo}")
}

/// Interpret a stored auto-sync preference. Default on — only an
/// explicit `"off"` pauses, so a missing or unrecognized value
/// leaves background sync running.
fn pref_is_enabled(stored: Option<&str>) -> bool {
    stored != Some("off")
}

/// Whether background sync is enabled for `repo` (default on).
pub fn is_enabled(repo: &str) -> bool {
    let stored = window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(&pref_key(repo)).ok().flatten());
    pref_is_enabled(stored.as_deref())
}

/// Persist whether background sync is enabled for `repo`, returning
/// whether the preference was actually written. The running
/// controller reads this fresh on its next sweep, so the change
/// takes effect without re-mounting.
///
/// A `false` return means the write did not land (localStorage
/// unavailable or rejected, e.g. quota or a privacy mode). Callers
/// must not show the new state as in effect when that happens — the
/// controller still reads the old, unchanged preference.
#[must_use]
pub fn set_enabled(repo: &str, enabled: bool) -> bool {
    let Ok(Some(storage)) = window().local_storage() else {
        return false;
    };
    let value = if enabled { "on" } else { "off" };
    storage.set_item(&pref_key(repo), value).is_ok()
}

#[wasm_bindgen]
extern "C" {
    /// Register a one-shot background sync under `tag` via the page's
    /// `self.tonkRegisterSync` (defined in `index.html`). Resolves once
    /// the user agent has accepted the registration; rejects where the
    /// Background Sync API is unavailable (Safari, Firefox) or
    /// registration fails, which routes the caller to the in-page
    /// sweep.
    #[wasm_bindgen(js_namespace = window, catch)]
    async fn tonkRegisterSync(tag: &str) -> Result<JsValue, JsValue>;
}

/// The background-sync tag for `repo`. The worker parses the repo back
/// out of it ([`tonk_worker::repo_from_sync_tag`]); the identity has
/// to ride in the tag because a `sync` event delivers only a string.
fn sync_tag(repo: &str) -> String {
    format!("tonk-sync:{repo}")
}

/// Whether this browser offers one-shot Background Sync. Chromium
/// does; Safari and Firefox don't, and there the commit path uses the
/// in-page sweep instead. A seam so [`commit_action`] is unit-testable
/// without a browser.
fn sync_manager_available() -> bool {
    js_sys::Reflect::has(&window(), &JsValue::from_str("SyncManager")).unwrap_or(false)
}

/// What the debounced post-commit trigger should do, given whether
/// auto-sync is enabled for the repo and whether the Background Sync
/// API is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitAction {
    /// Auto-sync is paused — do nothing.
    Skip,
    /// Register a durable one-shot background sync; the user agent
    /// owns the retry from there, even after the tab is gone.
    Register,
    /// Run the in-page sweep now — the polyfill where the Background
    /// Sync API is absent.
    Sweep,
}

/// Decide the post-commit action. Pure so the enabled/disabled and
/// available/absent branches are testable without a browser.
fn commit_action(enabled: bool, sync_manager_available: bool) -> CommitAction {
    if !enabled {
        CommitAction::Skip
    } else if sync_manager_available {
        CommitAction::Register
    } else {
        CommitAction::Sweep
    }
}

/// What a background trigger (interval, `online`, `visibilitychange`)
/// should do, given whether auto-sync is enabled for the repo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SweepAction {
    /// Auto-sync is on — pull then push every upstream branch.
    Sync,
    /// Auto-sync is paused — don't touch local or remote state, but
    /// still fetch the upstream head read-only so the sync-state badges
    /// keep reflecting where local sits relative to remote.
    RefreshStatus,
}

/// Decide what a background trigger should do. Pure so the
/// enabled/paused branches are testable without a browser.
fn sweep_action(enabled: bool) -> SweepAction {
    if enabled {
        SweepAction::Sync
    } else {
        SweepAction::RefreshStatus
    }
}

/// Ask the active repository's branch rows to re-read their read-only
/// sync status, by dispatching [`STATUS_REFRESH_EVENT`] on `window`
/// with `repo` in the event detail. The rows perform the actual
/// upstream fetch (via the read-only `sync/status` route), so a paused
/// repository still learns when remote moves out from under it.
fn request_status_refresh(repo: &str) {
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&JsValue::from_str(repo));
    let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(STATUS_REFRESH_EVENT, &init)
    else {
        return;
    };
    let _ = window().dispatch_event(&event);
}

/// Notify the background controller that a local commit just landed,
/// so it can sync shortly after. Called from the editor's commit
/// path; a no-op if no controller is listening.
pub fn notify_committed() {
    let Ok(event) = web_sys::CustomEvent::new(COMMITTED_EVENT) else {
        return;
    };
    let _ = window().dispatch_event(&event);
}

/// Wire the background sync triggers for the active repository.
///
/// Call once from the component that owns the active-repository
/// signal. The interval and event listeners are registered under
/// the current reactive owner, so they're torn down automatically
/// when that component unmounts.
///
/// A sweep reconciles upstream but does *not* refetch any UI
/// resource: subscribed components update from the worker's
/// subscription re-poll, and the branch rows refresh their revision
/// and sync-state badges off the per-branch broadcast the `/sync`
/// route posts (see [`crate::components::space`]). Refetching here
/// would tear down in-flight editor state on every tick.
pub fn mount(source: Signal<Option<String>, LocalStorage>) {
    // One sweep at a time — a slow sync must not stack ticks.
    let syncing = RwSignal::new(false);

    let sweep = move || {
        if syncing.get_untracked() {
            return;
        }
        let Some(repo) = source.get_untracked() else {
            return;
        };
        // Honor the per-repository pause preference, read fresh so a
        // toggle takes effect on the very next trigger. Paused stops
        // pull/push, but we still refresh the read-only sync status so
        // the badges keep tracking remote drift.
        match sweep_action(is_enabled(&repo)) {
            SweepAction::RefreshStatus => request_status_refresh(&repo),
            SweepAction::Sync => {
                syncing.set(true);
                spawn_local(async move {
                    sweep_repository(&repo).await;
                    // Refresh the status badges after the sweep too, not
                    // just on the paused tick, so a chip that reads
                    // status off this event (rather than the SSE `watch`
                    // layer) stays current. Harmless to the inspector's
                    // branch rows, which de-dupe on `state` equality.
                    request_status_refresh(&repo);
                    syncing.set(false);
                });
            }
        }
    };

    // Steady interval.
    use_interval_fn(sweep, TICK_INTERVAL_MS);

    // Back online — reconcile whatever drifted while offline.
    let _ = use_event_listener(window(), ev::online, move |_| sweep());

    // Tab refocused — pick up changes made elsewhere.
    let _ = use_event_listener(document(), ev::visibilitychange, move |_| {
        if document().visibility_state() == web_sys::VisibilityState::Visible {
            sweep();
        }
    });

    // Local commit — debounced so a burst of edits collapses into one
    // action. Where the Background Sync API is present we register a
    // durable one-shot sync so the push survives the tab closing or
    // going offline; otherwise we run the in-page sweep as the
    // polyfill. The interval, `online`, and `visibilitychange` triggers
    // above are untouched and keep running the in-page sweep regardless
    // of `SyncManager` support — they cover pull and double as a push
    // safety net.
    let on_commit = move || {
        let Some(repo) = source.get_untracked() else {
            return;
        };
        match commit_action(is_enabled(&repo), sync_manager_available()) {
            CommitAction::Skip => {}
            CommitAction::Sweep => sweep(),
            CommitAction::Register => {
                spawn_local(async move {
                    if tonkRegisterSync(&sync_tag(&repo)).await.is_err() {
                        // Unsupported or registration rejected — fall
                        // back to the in-page sweep so the commit still
                        // reconciles before the next tick.
                        sweep();
                    }
                });
            }
        }
    };
    let debounced = use_debounce_fn(on_commit, COMMIT_DEBOUNCE_MS);
    let _ = use_event_listener(
        window(),
        ev::Custom::<web_sys::Event>::new(COMMITTED_EVENT),
        move |_| {
            debounced();
        },
    );
}

/// Fetch the repository's current branch set and sync every branch
/// that has an upstream. Failures are logged, not surfaced — a
/// background sweep must never interrupt the user.
async fn sweep_repository(repo: &str) {
    let info = match api::repository(repo).await {
        Ok(Some(info)) => info,
        Ok(None) => return,
        Err(err) => {
            log!("background sync: could not load '{repo}': {err}");
            return;
        }
    };
    for branch in tonk_worker::branches_to_sync(&info.branch) {
        // The `/sync` route reports pull/push failures as a 200 with
        // `success: false` (a non-fast-forward push after divergence,
        // a fetch failure), so a transport-level `Ok` is not proof the
        // sync landed — inspect `success` too. Background failures are
        // logged, never surfaced, but they must not vanish silently.
        match api::sync(repo, &branch).await {
            Ok(response) if !response.success => {
                let detail = response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string());
                log!("background sync of {repo}/{branch} did not complete: {detail}");
            }
            Ok(_) => {}
            Err(err) => log!("background sync of {repo}/{branch} failed: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_skips_the_commit_action_when_auto_sync_is_paused() {
        assert_eq!(commit_action(false, true), CommitAction::Skip);
        assert_eq!(commit_action(false, false), CommitAction::Skip);
    }

    #[dialog_common::test]
    fn it_registers_a_background_sync_when_enabled_and_supported() {
        assert_eq!(commit_action(true, true), CommitAction::Register);
    }

    #[dialog_common::test]
    fn it_falls_back_to_the_in_page_sweep_when_supported_api_is_absent() {
        assert_eq!(commit_action(true, false), CommitAction::Sweep);
    }

    #[dialog_common::test]
    fn it_syncs_on_a_background_trigger_when_enabled() {
        assert_eq!(sweep_action(true), SweepAction::Sync);
    }

    #[dialog_common::test]
    fn it_refreshes_status_only_on_a_background_trigger_when_paused() {
        assert_eq!(sweep_action(false), SweepAction::RefreshStatus);
    }

    #[dialog_common::test]
    fn it_builds_a_sync_tag_the_worker_can_parse() {
        assert_eq!(sync_tag("home"), "tonk-sync:home");
    }

    #[dialog_common::test]
    fn it_defaults_to_enabled_when_no_preference_is_stored() {
        assert!(pref_is_enabled(None));
    }

    #[dialog_common::test]
    fn it_is_disabled_only_for_an_explicit_off() {
        assert!(!pref_is_enabled(Some("off")));
    }

    #[dialog_common::test]
    fn it_stays_enabled_for_on_or_unrecognized_values() {
        assert!(pref_is_enabled(Some("on")));
        assert!(pref_is_enabled(Some("anything-else")));
    }
}
