//! Background sync controller.
//!
//! Drives automatic reconciliation of the active repository's
//! upstream branches so a local write reaches the remote — and
//! remote changes reach the tab — without anyone clicking Pull or
//! Push. The manual buttons in [`crate::components::space`] stay as
//! they are; this sits on top of the same `/sync` routes.
//!
//! Every trigger funnels through one coordinator, [`run`], tagged with a
//! named [`Trigger`] reason and logged so you can always tell what fired a
//! sync (or status check) and why. The triggers:
//!
//! - [`Trigger::OnLoad`] — the active repo became known (load / navigation);
//! - [`Trigger::Interval`] — a steady [`TICK_INTERVAL_MS`] heartbeat;
//! - [`Trigger::Online`] — the window came back `online`;
//! - [`Trigger::Refocus`] — the tab became visible again;
//! - [`Trigger::LocalCommit`] — a local commit landed, debounced by
//!   [`COMMIT_DEBOUNCE_MS`] so a burst of edits collapses into one pass.
//!
//! Each pass does one of two things, decided by the per-repository pause
//! preference: a full **sync** (pull then push every upstream branch, then a
//! status check) when enabled, or a **status-only** check when paused (so the
//! chip keeps tracking remote drift without touching local/remote state). A
//! `LocalCommit` additionally registers a durable Background-Sync tag
//! (`tonk-sync:{repo}`) so the push survives the tab closing, where the
//! browser supports it.
//!
//! On by default for every repository; an explicit per-repository `off`
//! preference pauses it (see [`is_enabled`] / [`set_enabled`]).

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

/// Why the controller is running this pass. Every sync/status action
/// flows through [`run`] tagged with one of these, and is logged
/// (`sync[{reason}] …`) so you can always tell what fired it and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    /// The active repository just became known (load / navigation).
    OnLoad,
    /// The steady [`TICK_INTERVAL_MS`] heartbeat.
    Interval,
    /// The window came back `online`.
    Online,
    /// The tab became visible again.
    Refocus,
    /// A local commit landed (debounced). This is the only trigger that
    /// also registers a durable background sync.
    LocalCommit,
}

impl Trigger {
    /// Short tag used in log lines: `sync[interval] …`.
    fn tag(self) -> &'static str {
        match self {
            Trigger::OnLoad => "on-load",
            Trigger::Interval => "interval",
            Trigger::Online => "online",
            Trigger::Refocus => "refocus",
            Trigger::LocalCommit => "local-commit",
        }
    }
}

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

/// Whether this browser offers one-shot Background Sync. Chromium does;
/// Safari and Firefox don't, and there a `LocalCommit` skips the durable
/// registration and just syncs in-page.
fn sync_manager_available() -> bool {
    js_sys::Reflect::has(&window(), &JsValue::from_str("SyncManager")).unwrap_or(false)
}

/// Run one read-only status check for `repo`'s main branch: classify local
/// vs upstream and stamp the `tonk:sync` `state:here` overlay the chip
/// subscribes to. Logs the resulting state.
async fn check_status(repo: &str) {
    match api::sync_status(repo, "main").await {
        Ok(status) => log!("sync[status] {repo} → {:?}", status.state),
        Err(err) => log!("sync[status] {repo} check failed: {err}"),
    }
}

/// Notify the controller that a local commit just landed, so it can sync
/// shortly after. Called from the editor's commit path; a no-op if no
/// controller is listening.
pub fn notify_committed() {
    let Ok(event) = web_sys::CustomEvent::new(COMMITTED_EVENT) else {
        return;
    };
    let _ = window().dispatch_event(&event);
}

/// Per-mount state shared by every trigger: the active repo source and the
/// one-sweep-at-a-time guard.
#[derive(Clone, Copy)]
struct Controller {
    source: Signal<Option<String>, LocalStorage>,
    /// True while a sync is in flight — a slow sync must not stack ticks.
    busy: RwSignal<bool>,
}

impl Controller {
    /// THE single entry point. Every trigger calls this with its reason; it
    /// logs what it's doing and why, then either runs a full sync (enabled)
    /// or a status-only check (paused). `LocalCommit` additionally registers
    /// a durable background sync. Re-entrancy is guarded so overlapping
    /// triggers don't stack syncs.
    fn run(self, trigger: Trigger) {
        let Some(repo) = self.source.get_untracked() else {
            log!("sync[{}] no active repo — skipped", trigger.tag());
            return;
        };
        let enabled = is_enabled(&repo);

        // OnLoad is a status check only — it shouldn't push on every
        // navigation; the heartbeat and commit triggers drive the actual
        // sync. Paused repos likewise only check status.
        if trigger == Trigger::OnLoad || !enabled {
            log!(
                "sync[{}] {repo} → status check ({})",
                trigger.tag(),
                if enabled { "enabled" } else { "paused" }
            );
            spawn_local(async move { check_status(&repo).await });
            return;
        }

        // A local commit registers a durable background sync where supported,
        // so the push survives the tab closing — then still runs an in-page
        // sync now. Other triggers just sync in-page.
        if trigger == Trigger::LocalCommit && sync_manager_available() {
            let tag = sync_tag(&repo);
            log!("sync[local-commit] {repo} → register background sync '{tag}'");
            spawn_local(async move {
                if let Err(err) = tonkRegisterSync(&tag).await {
                    log!(
                        "sync[local-commit] background register rejected ({err:?}); syncing in-page"
                    );
                }
            });
        }

        if self.busy.get_untracked() {
            log!("sync[{}] {repo} → already syncing, skipped", trigger.tag());
            return;
        }
        log!("sync[{}] {repo} → sync (pull+push)", trigger.tag());
        self.busy.set(true);
        spawn_local(async move {
            sweep_repository(&repo).await;
            check_status(&repo).await;
            self.busy.set(false);
        });
    }
}

/// Wire the sync triggers for the active repository.
///
/// Call once from the component that owns the active-repository signal. The
/// interval and event listeners register under the current reactive owner,
/// so they tear down when that component unmounts. Every trigger funnels
/// through [`Controller::run`] tagged with its [`Trigger`] reason and logged.
pub fn mount(source: Signal<Option<String>, LocalStorage>) {
    let ctl = Controller {
        source,
        busy: RwSignal::new(false),
    };

    // Steady heartbeat.
    use_interval_fn(move || ctl.run(Trigger::Interval), TICK_INTERVAL_MS);

    // Back online — reconcile whatever drifted while offline.
    let _ = use_event_listener(window(), ev::online, move |_| ctl.run(Trigger::Online));

    // Tab refocused — pick up changes made elsewhere.
    let _ = use_event_listener(document(), ev::visibilitychange, move |_| {
        if document().visibility_state() == web_sys::VisibilityState::Visible {
            ctl.run(Trigger::Refocus);
        }
    });

    // Local commit — debounced so a burst of edits collapses into one pass.
    let debounced = use_debounce_fn(move || ctl.run(Trigger::LocalCommit), COMMIT_DEBOUNCE_MS);
    let _ = use_event_listener(
        window(),
        ev::Custom::<web_sys::Event>::new(COMMITTED_EVENT),
        move |_| {
            debounced();
        },
    );

    // The active repo became known (load) — and re-runs on navigation when
    // `source` changes — so the chip shows a real state at once instead of
    // waiting for the first heartbeat (the status overlay is transient).
    Effect::new(move |_| {
        if source.get().is_some() {
            ctl.run(Trigger::OnLoad);
        }
    });
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
    fn it_tags_each_trigger_for_logging() {
        assert_eq!(Trigger::Interval.tag(), "interval");
        assert_eq!(Trigger::LocalCommit.tag(), "local-commit");
        assert_eq!(Trigger::OnLoad.tag(), "on-load");
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
