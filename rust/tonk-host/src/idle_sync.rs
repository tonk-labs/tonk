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
//! Only the top page's installed host runs this. The sealed guest installs a
//! relay over `window.tonk`; its fetches already reach the top page (and thus
//! `on_fetch`) through the bridge, so one heartbeat per tab is enough.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{RequestInit, VisibilityState, window};

/// The parameterless drain endpoint every poll hits.
const SYNC_ENDPOINT: &str = "/api/sync";

/// How long after an idle poll before arming the next one, on a visible tab.
/// The heartbeat only covers the "tab open but making no requests" gap, so a
/// relaxed cadence suffices — refocus and reconnect poll immediately via
/// their own listeners, and any real traffic schedules the SW drain anyway.
/// (Was 1s, which read as non-stop `/api/sync` chatter in the network panel.)
const IDLE_REARM_MS: i32 = 10_000;

/// A handle owning the installed idle-sync listeners and loop. Held for the
/// page's lifetime by the installed host.
pub(crate) struct IdleSync {
    /// Kept so the self-rescheduling idle loop's teardown flag stays
    /// reachable should teardown ever return; never flipped today (the host
    /// installs once for the page's lifetime).
    _disposed: Rc<Cell<bool>>,
    /// Kept alive so the listeners stay registered.
    _visibility: Closure<dyn FnMut()>,
    _online: Closure<dyn FnMut()>,
    _offline: Closure<dyn FnMut()>,
}

/// Fire one `POST /api/sync?why=<trigger>`, fire-and-forget. `why` names the
/// trigger (`idle` / `visible` / `online` / `offline`) so the network panel —
/// and the SW's drain-cause log — show what initiated each poll. Failures
/// are ignored: a missed poll just means the next one (or ordinary traffic)
/// reconciles.
///
/// NOT guarded on connectivity here: the request is SW-served (never
/// reaches the network), and the `offline` trigger exists precisely to
/// reach the SW while offline so it stamps `sync:offline`. The steady
/// triggers guard at their call sites via [`online`].
fn poll(why: &str) {
    let Some(win) = window() else { return };
    let init = RequestInit::new();
    init.set_method("POST");
    let _ = win.fetch_with_str_and_init(&format!("{SYNC_ENDPOINT}?why={why}"), &init);
}

/// Whether the browser reports connectivity. The idle loop and refocus
/// polls skip while offline — there is no upstream to pull from, and the
/// `online`/`offline` listeners poll on each transition.
fn online() -> bool {
    window().map(|w| w.navigator().on_line()).unwrap_or(true)
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
        if online() {
            poll("idle");
        }
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
                && online()
            {
                poll("visible");
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
                poll("online");
            }
        }) as Box<dyn FnMut()>)
    };
    win.add_event_listener_with_callback("online", online.as_ref().unchecked_ref())
        .ok()?;

    // One poll when connectivity is LOST: the request is SW-served, so it
    // still lands, and it is the only thing that wakes the SW to stamp
    // `sync:offline` — with the steady polls paused, no other request
    // would reach it until connectivity returns.
    let offline = {
        let disposed = disposed.clone();
        Closure::wrap(Box::new(move || {
            if !disposed.get() {
                poll("offline");
            }
        }) as Box<dyn FnMut()>)
    };
    win.add_event_listener_with_callback("offline", offline.as_ref().unchecked_ref())
        .ok()?;

    // Kick the steady idle loop.
    arm(disposed.clone());

    Some(IdleSync {
        _disposed: disposed,
        _visibility: visibility,
        _online: online,
        _offline: offline,
    })
}
