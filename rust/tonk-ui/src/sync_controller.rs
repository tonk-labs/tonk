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
//! `off` preference pauses it (see [`is_enabled`] / [`set_enabled`]),
//! leaving only the manual Pull/Push buttons.

use std::collections::HashMap;

use leptos::ev;
use leptos::logging::log;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_use::{use_debounce_fn, use_event_listener, use_interval_fn};
use tonk_worker::{BranchConfiguration, RepositoryInfo};

use crate::api;
use crate::error::TonkUiError;

/// How often the controller sweeps the active repository's branches.
/// One knob, tuned in one place.
const TICK_INTERVAL_MS: u64 = 20_000;

/// Debounce window after a local commit before syncing — coalesces
/// a burst of writes into one sweep.
const COMMIT_DEBOUNCE_MS: f64 = 1_000.0;

/// DOM event a successful local commit dispatches on `window`. The
/// controller listens for it to sync shortly after a write.
pub const COMMITTED_EVENT: &str = "tonk:committed";

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

/// Branch names in `branches` that have an upstream, sorted for a
/// stable sweep order. Branches without an upstream have nowhere to
/// sync to, so they're skipped.
pub fn branches_to_sync(branches: &HashMap<String, BranchConfiguration>) -> Vec<String> {
    let mut names: Vec<String> = branches
        .iter()
        .filter(|(_, config)| config.upstream.is_some())
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names
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
/// when that component unmounts. After each sweep the `repository`
/// resource is refetched so revisions in the view reflect the new
/// state.
pub fn mount(
    source: Signal<Option<String>, LocalStorage>,
    repository: LocalResource<Result<Option<RepositoryInfo>, TonkUiError>>,
) {
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
        // toggle takes effect on the very next trigger. Paused means
        // only the manual Pull/Push buttons act.
        if !is_enabled(&repo) {
            return;
        }
        syncing.set(true);
        spawn_local(async move {
            sweep_repository(&repo).await;
            repository.refetch();
            syncing.set(false);
        });
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

    // Local commit — debounced so a burst of edits syncs once.
    let debounced = use_debounce_fn(sweep, COMMIT_DEBOUNCE_MS);
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
    for branch in branches_to_sync(&info.branch) {
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
    use tonk_worker::UpstreamConfiguration;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn with_upstream() -> BranchConfiguration {
        BranchConfiguration {
            upstream: Some(UpstreamConfiguration {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            revision: None,
        }
    }

    fn without_upstream() -> BranchConfiguration {
        BranchConfiguration {
            upstream: None,
            revision: None,
        }
    }

    #[dialog_common::test]
    fn it_selects_only_branches_with_an_upstream() {
        let branches = HashMap::from([
            ("main".to_string(), with_upstream()),
            ("scratch".to_string(), without_upstream()),
        ]);
        assert_eq!(branches_to_sync(&branches), vec!["main".to_string()]);
    }

    #[dialog_common::test]
    fn it_returns_branches_sorted_for_a_stable_sweep_order() {
        let branches = HashMap::from([
            ("zeta".to_string(), with_upstream()),
            ("alpha".to_string(), with_upstream()),
        ]);
        assert_eq!(
            branches_to_sync(&branches),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[dialog_common::test]
    fn it_returns_empty_when_no_branch_has_an_upstream() {
        let branches = HashMap::from([("main".to_string(), without_upstream())]);
        assert!(branches_to_sync(&branches).is_empty());
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
