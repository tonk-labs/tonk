//! Redirecting the page that asked — the worker side of a navigation.
//!
//! A command handler whose effect is a page capability (loading a new
//! location) can't perform it itself: the service worker has no
//! `window`, and a transient command never lands in a branch a
//! subscription could observe. The only channel back to the page that
//! asked is a `postMessage` to its originating client; the page's
//! `<tonk-host>` listens for `navigate` messages and assigns
//! `window.location`. Used by the join handler (redirect into the
//! joined space) and the create handler (drop the creator into the
//! fresh spot).

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;

/// Post a `{ type: "navigate", href }` message to the originating client so
/// it redirects there.
///
/// No-ops (with a log) when the client is unknown or its handle can't be
/// resolved — the triggering command still succeeded; only the convenience
/// redirect is lost, and the user can navigate from the Hub.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn notify_navigate(client: Option<&crate::router::ClientId>, href: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let Some(client) = client else {
        log!("navigate: no originating client; skipping redirect");
        return;
    };
    let client_id = client.0.clone();
    let href = href.to_owned();

    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(g) => g,
        Err(_) => {
            log!("navigate: not in a service worker scope; skipping redirect");
            return;
        }
    };

    // `clients.get(id)` resolves the live `Client` handle; post the message
    // on it. Done on a spawned task so the caller isn't blocked on the
    // round-trip (the navigate is fire-and-forget).
    spawn_local(async move {
        let client_value = match JsFuture::from(global.clients().get(&client_id)).await {
            Ok(value) if !value.is_undefined() && !value.is_null() => value,
            Ok(_) => {
                log!("navigate: originating client {client_id} is gone; skipping redirect");
                return;
            }
            Err(e) => {
                log!("navigate: clients.get failed: {e:?}");
                return;
            }
        };
        let Ok(client) = client_value.dyn_into::<web_sys::Client>() else {
            log!("navigate: clients.get did not yield a Client; skipping redirect");
            return;
        };

        // `{ type: "navigate", href }` — the page's `<tonk-host>` listens
        // for `navigate` messages and assigns `window.location`.
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("navigate"),
        );
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("href"),
            &JsValue::from_str(&href),
        );
        if let Err(e) = client.post_message(&message) {
            log!("navigate: post_message(navigate) failed: {e:?}");
        }
    });
}

/// Ask the originating top document to provision a local root and replay an intent.
/// The intent is never formatted or logged because durable-join intents may contain
/// an authority-bearing invite URL.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn notify_identity_required(
    client: Option<&crate::router::ClientId>,
    intent: tonk_worker_api::IdentityIntent,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let Some(client) = client else {
        log!("identity request has no originating client");
        return;
    };
    let client_id = client.0.clone();
    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(global) => global,
        Err(_) => return,
    };
    spawn_local(async move {
        let Ok(value) = JsFuture::from(global.clients().get(&client_id)).await else {
            return;
        };
        let Ok(client) = value.dyn_into::<web_sys::Client>() else {
            return;
        };
        let message = tonk_worker_api::IdentityRequired {
            message_type: "identity-required".to_string(),
            intent,
        };
        let Ok(message) = serde_wasm_bindgen::to_value(&message) else {
            return;
        };
        let _ = client.post_message(&message);
    });
}
