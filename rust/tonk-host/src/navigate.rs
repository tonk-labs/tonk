//! Main-thread navigate provider for `<tonk-host>`.
//!
//! A worker-side command (e.g. `tonk:join` on success) can't perform a
//! navigation itself: the service worker has no `window`, and a transient
//! command never lands in a branch a subscription could observe. So the
//! worker posts a `{ type: "navigate", href }` message to the originating
//! client, and this listener — installed on `navigator.serviceWorker` by
//! `<tonk-host>` — performs the redirect with `window.location.assign`.
//!
//! This is the page-side half of Elm's `pushUrl`, routed through the
//! platform's worker→page channel rather than through branch state. It is
//! the first "main-thread command provider"; when a second page-only effect
//! appears (clipboard, focus, title), generalize this into a small registry
//! keyed by message `type`.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CustomEvent, CustomEventInit, MessageEvent, window};

/// A handle owning the installed `message` listener. Dropping it (or calling
/// [`remove`](NavigateListener::remove)) detaches the listener.
pub(crate) struct NavigateListener {
    closure: Closure<dyn FnMut(MessageEvent)>,
}

impl NavigateListener {
    /// Detach the listener from `navigator.serviceWorker`.
    pub(crate) fn remove(self) {
        if let Some(container) = service_worker_container() {
            let _ = container.remove_event_listener_with_callback(
                "message",
                self.closure.as_ref().unchecked_ref(),
            );
        }
    }
}

/// Install a `navigator.serviceWorker` `message` listener that handles
/// worker→page messages:
/// - `{ type: "navigate", href }` — assigns `window.location`.
/// - `{ type: "sync" }` — dispatches `tonk:committed` on `window` so the
///   sync controller pushes immediately instead of waiting for the heartbeat.
///
/// Returns `None` when there is no service-worker container (e.g. a
/// non-secure context or a test stub).
pub(crate) fn install() -> Option<NavigateListener> {
    let container = service_worker_container()?;
    let closure = Closure::wrap(Box::new(move |event: MessageEvent| {
        let data = event.data();
        if let Some(href) = navigate_href(&data) {
            navigate_to(&href);
        } else if is_sync_message(&data) {
            dispatch_committed();
        }
    }) as Box<dyn FnMut(MessageEvent)>);
    container
        .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
        .ok()?;
    Some(NavigateListener { closure })
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

/// Assign `window.location` to `href`, redirecting the page.
fn navigate_to(href: &str) {
    if let Some(location) = window().map(|w| w.location()) {
        let _ = location.assign(href);
    }
}

/// The page's `navigator.serviceWorker` container, if available.
fn service_worker_container() -> Option<web_sys::ServiceWorkerContainer> {
    Some(window()?.navigator().service_worker())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
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
}
