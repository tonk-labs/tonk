//! Main-thread navigate provider, installed with the host.
//!
//! A worker-side command (e.g. `tonk:join` on success) can't perform a
//! navigation itself: the service worker has no `window`, and a transient
//! command never lands in a branch a subscription could observe. So the
//! worker posts a `{ type: "navigate", href }` message to the originating
//! client, and this listener — installed on `navigator.serviceWorker` at
//! host install — performs the redirect with `window.location.assign`.
//!
//! This is the page-side half of Elm's `pushUrl`, routed through the
//! platform's worker→page channel rather than through branch state. It is
//! the first "main-thread command provider"; when a second page-only effect
//! appears (clipboard, focus, title), generalize this into a small registry
//! keyed by message `type`.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CustomEvent, CustomEventInit, MessageEvent, window};

/// A handle owning the installed `message` listener, held for the page's
/// lifetime by the installed host.
pub(crate) struct NavigateListener {
    _closure: Closure<dyn FnMut(MessageEvent)>,
}

/// Install a `navigator.serviceWorker` `message` listener that handles
/// worker→page messages:
/// - `{ type: "navigate", href }` — assigns `window.location`.
/// - `{ type: "sync" }` — dispatches `tonk:committed` on `window` so the
///   sync controller pushes immediately instead of waiting for the heartbeat.
/// - `{ type: "profile-changed" }` — reloads the top-level document so it
///   receives a fresh service-worker client binding for the active profile.
///
/// Returns `None` when there is no service-worker container (e.g. a
/// non-secure context or a test stub).
pub(crate) fn install() -> Option<NavigateListener> {
    let container = service_worker_container()?;
    let closure = Closure::wrap(Box::new(move |event: MessageEvent| {
        handle_worker_message(&event.data());
    }) as Box<dyn FnMut(MessageEvent)>);
    container
        .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
        .ok()?;
    Some(NavigateListener { _closure: closure })
}

fn handle_worker_message(data: &JsValue) {
    if let Some(href) = navigate_href(data) {
        navigate_to(&href);
    } else if is_sync_message(data) {
        dispatch_committed();
    } else if is_profile_changed_message(data) {
        reload_page();
    }
}

/// Read `href` out of a `{ type: "navigate", href }` message, or `None` when
/// the message isn't a navigate or carries no usable href.
fn navigate_href(data: &JsValue) -> Option<String> {
    let kind = js_sys::Reflect::get(data, &JsValue::from_str("type"))
        .ok()?
        .as_string()?;
    if kind != "navigate" {
        return None;
    }
    let href = js_sys::Reflect::get(data, &JsValue::from_str("href"))
        .ok()?
        .as_string()?;
    if href.is_empty() {
        return None;
    }
    Some(href)
}

/// Return `true` for a `{ type: "sync" }` message.
fn is_sync_message(data: &JsValue) -> bool {
    js_sys::Reflect::get(data, &JsValue::from_str("type"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|kind| kind == "sync")
        .unwrap_or(false)
}

/// Return `true` for an identifier-free `{ type: "profile-changed" }`
/// message from the worker.
fn is_profile_changed_message(data: &JsValue) -> bool {
    js_sys::Reflect::get(data, &JsValue::from_str("type"))
        .ok()
        .and_then(|value| value.as_string())
        .is_some_and(|kind| kind == "profile-changed")
}

/// Dispatch `tonk:committed` on `window` so the sync controller treats it
/// like a local commit and pushes promptly. Keeps tonk-host decoupled from
/// tonk-ui — no cross-crate dependency needed.
fn dispatch_committed() {
    let Some(win) = window() else { return };
    let init = CustomEventInit::new();
    init.set_bubbles(false);
    init.set_cancelable(false);
    if let Ok(event) = CustomEvent::new_with_event_init_dict("tonk:committed", &init) {
        let _ = win.dispatch_event(&event);
    }
}

/// Ask the host page to raise its registration dialog.
///
/// Sharing needs an account and only the top page can run the ceremony,
/// so a guest that hits a `needs-account` refusal forwards the ask
/// rather than trying to register where it cannot. A page with no
/// bridge (the shell itself) does nothing: it would already have raised
/// the dialog directly.
///
/// `payload` is `{"reason": <refusal class>, "space": <did>}` as JSON.
/// The reason words the prompt; the space is what the dialog shares once
/// an account exists, so the click that was interrupted still ends in a
/// link. A single string because [`crate::page_effect::forward`] carries
/// one argument for every effect.
pub fn request_registration(payload: &str) {
    crate::page_effect::forward("register", payload);
}

/// Navigate to `href` WITHOUT reloading: push it onto history and fire
/// `popstate` so the top-level `<tonk-site>` re-resolves. The path change then
/// updates the tab's site in the overlay, whose subscription re-renders the
/// view — the route change propagates as a data change, not a page load. Falls
/// back to a real `location.assign` only if history isn't available.
///
/// In a guest this forwards to the parent instead (see `page_effect`): a
/// guest's document is `about:srcdoc` at an opaque origin, where `pushState`
/// to a real URL throws and the `location.assign` fallback below would load
/// the whole app INSIDE the iframe.
///
/// Public: the portal bridge performs a guest's relayed link click through
/// this too, so an in-guest navigation stays a client-side route change.
pub fn navigate_to(href: &str) {
    use wasm_bindgen::JsValue;
    if crate::page_effect::forward("navigate", href) {
        return;
    }
    let Some(win) = window() else {
        return;
    };

    // Navigating to where we already are is a no-op, not a navigation. Without
    // this guard every such call pushed a DUPLICATE history entry and fired a
    // `popstate` that re-routed the site and re-stamped it for no change — and
    // the duplicates then had to be walked back through one by one on Back.
    // Resolved against the current document so a relative `href` compares
    // correctly with `location`.
    if let Ok(current) = win.location().href()
        && let Ok(target) = web_sys::Url::new_with_base(href, &current)
        && target.href() == current
    {
        return;
    }

    let pushed = win
        .history()
        .ok()
        .map(|h| {
            h.push_state_with_url(&JsValue::NULL, "", Some(href))
                .is_ok()
        })
        .unwrap_or(false);
    if pushed {
        // `pushState` fires no event; dispatch `popstate` so listeners re-route.
        if let Ok(event) = web_sys::Event::new("popstate") {
            let _ = win.dispatch_event(&event);
        }
    } else {
        // No history access — fall back to a real (reloading) navigation.
        let _ = win.location().assign(href);
    }
}

/// Reload the top page, forwarding through every sealed guest boundary.
///
/// Unlike [`navigate_to`], this deliberately refreshes an unchanged route.
/// Account-profile activation swaps the service worker's entire active state;
/// rebuilding the page is what drops subscriptions owned by the old profile.
pub fn reload_page() {
    if crate::page_effect::forward("reload", "") {
        return;
    }
    if let Some(win) = window() {
        let _ = win.location().reload();
    }
}

/// The page's `navigator.serviceWorker` container, if available.
fn service_worker_container() -> Option<web_sys::ServiceWorkerContainer> {
    Some(window()?.navigator().service_worker())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn message(kind: &str, href: &str) -> JsValue {
        let object = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str(kind),
        );
        let _ = js_sys::Reflect::set(
            &object,
            &JsValue::from_str("href"),
            &JsValue::from_str(href),
        );
        object.into()
    }

    fn sync_message() -> JsValue {
        let object = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str("sync"),
        );
        object.into()
    }

    /// `navigate_href` accepts only a `{ type: "navigate", href }` shape with
    /// a non-empty href; everything else yields `None` (so an unrelated SW
    /// message never navigates). We assert the parse, not the navigation —
    /// performing it would tear the test harness out from under us.
    #[dialog_common::test]
    async fn it_reads_href_only_from_a_navigate_message() {
        assert_eq!(
            navigate_href(&message("navigate", "/space/abc")),
            Some("/space/abc".to_owned()),
            "a navigate message with an href should yield it"
        );
        assert_eq!(
            navigate_href(&message("navigate", "")),
            None,
            "an empty href should yield None"
        );
        assert_eq!(
            navigate_href(&message("other", "/space/abc")),
            None,
            "a non-navigate message should yield None"
        );
        assert_eq!(
            navigate_href(&JsValue::from_str("not an object")),
            None,
            "a non-object payload should yield None"
        );
    }

    /// `is_sync_message` accepts only `{ type: "sync" }`; navigate and
    /// unrelated messages yield `false`.
    #[dialog_common::test]
    async fn it_recognises_only_a_sync_message() {
        assert!(
            is_sync_message(&sync_message()),
            "a sync message should be recognised"
        );
        assert!(
            !is_sync_message(&message("navigate", "/space/abc")),
            "a navigate message must not be treated as sync"
        );
        assert!(
            !is_sync_message(&message("other", "")),
            "an unrelated message must not be treated as sync"
        );
        assert!(
            !is_sync_message(&JsValue::from_str("not an object")),
            "a non-object payload should not be recognised as sync"
        );
    }

    #[dialog_common::test]
    async fn it_reloads_for_a_profile_changed_worker_message() {
        let calls = install_effect_stub("reload");

        handle_worker_message(&message("profile-changed", ""));

        clear_tonk();
        assert_eq!(calls.length(), 1);
        assert_eq!(calls.get(0).as_string().as_deref(), Some(""));
    }

    #[dialog_common::test]
    async fn it_ignores_unrelated_worker_messages() {
        let calls = install_effect_stub("reload");

        handle_worker_message(&message("unrelated", ""));

        clear_tonk();
        assert_eq!(calls.length(), 0);
    }

    /// Install a stub `window.tonk[method]` recording its argument. See the
    /// note in `page_effect.rs`: `window` is shared across the whole wasm
    /// test module, so this MUST be cleared before the test returns.
    fn install_effect_stub(method: &str) -> Array {
        let calls = Array::new();
        let recorder = {
            let calls = calls.clone();
            Closure::wrap(Box::new(move |value: JsValue| {
                calls.push(&value);
            }) as Box<dyn FnMut(JsValue)>)
        };
        let tonk = Object::new();
        let _ = Reflect::set(
            &tonk,
            &JsValue::from_str(method),
            recorder.as_ref().unchecked_ref::<Function>(),
        );
        recorder.forget();
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
        calls
    }

    fn clear_tonk() {
        let win = window().expect("a window in the test harness");
        let _ = Reflect::delete_property(win.unchecked_ref::<Object>(), &JsValue::from_str("tonk"));
    }

    /// In a guest, `navigate_to` posts to the parent instead of touching this
    /// document's history. This is the one navigation we CAN assert directly:
    /// forwarding means nothing actually navigates, so the harness survives.
    #[dialog_common::test]
    async fn it_forwards_a_navigation_from_a_guest_instead_of_performing_it() {
        let before = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        let calls = install_effect_stub("navigate");

        navigate_to("/space/forwarded");

        let after = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        // Restore BEFORE asserting so a failure doesn't leak `window.tonk`
        // into every later test in this binary — a panic unwinds past any
        // cleanup placed after the assertions. `bridge.rs:2643` does exactly
        // this, for exactly this reason. `calls` is an independent handle, so
        // clearing the stub does not disturb what it already recorded.
        clear_tonk();

        assert_eq!(calls.length(), 1, "the parent should have been called once");
        assert_eq!(
            calls.get(0).as_string(),
            Some("/space/forwarded".to_owned()),
            "the href should reach the parent verbatim"
        );
        assert_eq!(
            before, after,
            "a forwarded navigation must not move this document"
        );
    }

    /// A profile switch initiated inside a sealed Hub must reload the top
    /// page, not the opaque guest document that requested it.
    #[dialog_common::test]
    async fn it_forwards_a_reload_from_a_guest_instead_of_reloading_it() {
        let before = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        let calls = install_effect_stub("reload");

        reload_page();

        let after = window()
            .expect("a window in the test harness")
            .location()
            .href()
            .expect("a location href");
        clear_tonk();

        assert_eq!(calls.length(), 1, "the parent should be called once");
        assert_eq!(calls.get(0).as_string().as_deref(), Some(""));
        assert_eq!(before, after, "a forwarded reload must spare this guest");
    }
}
