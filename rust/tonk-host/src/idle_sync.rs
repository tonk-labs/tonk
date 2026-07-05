//! Idle sync heartbeat for the top-page `<tonk-host>`.
//!
//! The service worker pulls upstream changes as a side effect of ordinary
//! request traffic: every `fetch` through its `on_fetch` schedules a debounced
//! drain. That premise holds while the page keeps making requests — but a space
//! that has finished booting and is only holding open SSE subscriptions makes no
//! *new* fetch, so no drain re-fires and remote edits never arrive.
//!
//! This closes that gap. While the tab is open it polls `POST /api/sync` on a
//! `requestIdleCallback` loop (re-arming every [`IDLE_REARM_MS`]), so an idle
//! viewer still pulls. It also polls immediately when the tab becomes visible
//! or comes back online, so a refocus reconciles at once rather than waiting for
//! the next tick. The poll does no work server-side — `/api/sync` returns `200`
//! and the drain is scheduled by `on_fetch` seeing the request — so each poll
//! participates in the SW's own debounce/coalescing rather than forcing a sync.
//!
//! `requestIdleCallback` is throttled to ~one call / 10s on a hidden tab (per
//! spec), so a backgrounded tab self-limits without any explicit gating. Where
//! `requestIdleCallback` is unavailable (Safari) it falls back to `setTimeout`.
//!
//! Only the real top-page `<tonk-host>` installs this. The sealed guest runs a
//! proxy host that relays over `window.tonk`; its fetches already reach the top
//! page (and thus `on_fetch`) through the bridge, so one heartbeat per tab is
//! enough.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{RequestInit, VisibilityState, window};

/// The parameterless drain endpoint every poll hits.
const SYNC_ENDPOINT: &str = "/api/sync";

/// How long after an idle poll before arming the next one, on a visible tab.
/// Twice the SW drain debounce: the debounce is the real rate-limiter, this
/// just keeps the pump primed.
const IDLE_REARM_MS: i32 = 1_000;

/// A handle owning the installed idle-sync listeners and loop. Dropping it (or
/// calling [`remove`](IdleSync::remove)) stops the `requestIdleCallback` loop
/// and detaches the `visibilitychange` / `online` listeners.
pub(crate) struct IdleSync {
    /// Flips true on teardown; the self-rescheduling idle loop checks it and
    /// stops instead of arming another callback.
    disposed: Rc<Cell<bool>>,
    /// Kept alive so the listeners stay registered; detached in [`remove`].
    visibility: Closure<dyn FnMut()>,
    online: Closure<dyn FnMut()>,
}

impl IdleSync {
    /// Detach the event listeners and stop the idle loop.
    pub(crate) fn remove(self) {
        self.disposed.set(true);
        if let Some(win) = window() {
            let doc = win.document();
            if let Some(doc) = doc {
                let _ = doc.remove_event_listener_with_callback(
                    "visibilitychange",
                    self.visibility.as_ref().unchecked_ref(),
                );
            }
            let _ = win.remove_event_listener_with_callback(
                "online",
                self.online.as_ref().unchecked_ref(),
            );
        }
    }
}

/// Fire one `POST /api/sync`, fire-and-forget. Failures are ignored: a missed
/// poll just means the next one (or ordinary traffic) reconciles.
fn poll() {
    let Some(win) = window() else { return };
    let init = RequestInit::new();
    init.set_method("POST");
    let _ = win.fetch_with_str_and_init(SYNC_ENDPOINT, &init);
}

/// Arm the next idle poll via `requestIdleCallback`, re-arming itself until the
/// host is disposed. Falls back to `setTimeout` where `requestIdleCallback` is
/// absent (Safari). Re-scheduling from inside the callback keeps exactly one
/// pending poll in flight; successive idle polls are spaced by
/// [`IDLE_REARM_MS`] so a visible idle tab doesn't busy-loop.
fn arm(disposed: Rc<Cell<bool>>) {
    if disposed.get() {
        return;
    }
    let Some(win) = window() else { return };

    let disposed_for_cb = disposed.clone();
    let callback = Closure::once_into_js(move || {
        if disposed_for_cb.get() {
            return;
        }
        poll();
        // Space the next idle poll by IDLE_REARM_MS, then arm another idle
        // callback. `requestIdleCallback` can fire back-to-back on a busy
        // visible tab, so this timer is what bounds the idle cadence.
        if let Some(win) = window() {
            let disposed = disposed_for_cb.clone();
            let next = Closure::once_into_js(move || arm(disposed));
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                next.unchecked_ref(),
                IDLE_REARM_MS,
            );
        }
    });

    // `requestIdleCallback` isn't in web-sys; reach it off `window` and fall
    // back to a `setTimeout` on the same cadence where it's missing.
    let ric = js_sys::Reflect::get(&win, &JsValue::from_str("requestIdleCallback")).ok();
    let scheduled = match ric {
        Some(f) if f.is_function() => js_sys::Reflect::apply(
            f.unchecked_ref(),
            &win,
            &js_sys::Array::of1(callback.unchecked_ref()),
        )
        .is_ok(),
        _ => false,
    };

    if !scheduled {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref(),
            IDLE_REARM_MS,
        );
    }
}

/// Install the idle-sync heartbeat: start the `requestIdleCallback` loop and
/// listen for `visibilitychange` / `online` to poll immediately on refocus or
/// reconnect. Returns `None` when there is no `window`/`document` (e.g. a test
/// stub), leaving the SW's ordinary-traffic drain as the only path.
pub(crate) fn install() -> Option<IdleSync> {
    let win = window()?;
    let doc = win.document()?;

    let disposed = Rc::new(Cell::new(false));

    // Immediate poll when the tab becomes visible again — a refocus reconciles
    // at once rather than waiting for the next idle tick.
    let visibility = {
        let disposed = disposed.clone();
        Closure::wrap(Box::new(move || {
            if disposed.get() {
                return;
            }
            if let Some(doc) = window().and_then(|w| w.document())
                && doc.visibility_state() == VisibilityState::Visible
            {
                poll();
            }
        }) as Box<dyn FnMut()>)
    };
    doc.add_event_listener_with_callback("visibilitychange", visibility.as_ref().unchecked_ref())
        .ok()?;

    // Immediate poll when connectivity returns.
    let online = {
        let disposed = disposed.clone();
        Closure::wrap(Box::new(move || {
            if !disposed.get() {
                poll();
            }
        }) as Box<dyn FnMut()>)
    };
    win.add_event_listener_with_callback("online", online.as_ref().unchecked_ref())
        .ok()?;

    // Kick the steady idle loop.
    arm(disposed.clone());

    Some(IdleSync {
        disposed,
        visibility,
        online,
    })
}
