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
//! fresh space).

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

/// Post a typed launch-funnel success to the originating page.
///
/// The message never leaves the browser. It carries the local space routing
/// key so the page can hash it at the analytics boundary before capture.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn notify_analytics(
    client: Option<&crate::router::ClientId>,
    event: tonk_worker_api::AnalyticsEvent,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let Some(client) = client else {
        log!("analytics: no originating client; skipping event");
        return;
    };
    let client_id = client.0.clone();
    let message = tonk_worker_api::AnalyticsMessage::new(event);

    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(global) => global,
        Err(_) => {
            log!("analytics: not in a service worker scope; skipping event");
            return;
        }
    };

    spawn_local(async move {
        let client_value = match JsFuture::from(global.clients().get(&client_id)).await {
            Ok(value) if !value.is_undefined() && !value.is_null() => value,
            Ok(_) => {
                log!("analytics: originating client {client_id} is gone; skipping event");
                return;
            }
            Err(error) => {
                log!("analytics: clients.get failed: {error:?}");
                return;
            }
        };
        let Ok(client) = client_value.dyn_into::<web_sys::Client>() else {
            log!("analytics: clients.get did not yield a Client; skipping event");
            return;
        };
        let message = match serde_wasm_bindgen::to_value(&message) {
            Ok(message) => message,
            Err(error) => {
                log!("analytics: failed to serialize event: {error}");
                return;
            }
        };
        if let Err(error) = client.post_message(&message) {
            log!("analytics: post_message failed: {error:?}");
        }
    });
}

/// Ask the originating document to run a WebAuthn ceremony the worker
/// cannot: it has no `window`. The page answers through the ordinary API
/// (`POST /api/identity/root`), which is what the worker then waits on.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
/// Ask the page to link an account, on behalf of `space`.
///
/// Sent when a share cannot proceed because nothing is registered. The
/// worker owns that judgement — the page is told what to do, not why —
/// and does not wait: the registration UI may take a ceremony, an email
/// round trip, or never finish, and a handler held open across that is
/// held open forever. The share resumes when the account facts land.
pub(crate) async fn request_account_link(
    client: &crate::router::ClientId,
    space: &str,
) -> Result<(), crate::TonkWorkerError> {
    use crate::TonkWorkerError;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service worker scope".to_string()))?;
    let value = JsFuture::from(global.clients().get(&client.0))
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("clients.get failed: {error:?}")))?;
    if value.is_undefined() || value.is_null() {
        return Err(TonkWorkerError::Conflict(format!(
            "the originating client {} is gone",
            client.0
        )));
    }
    let client: web_sys::Client = value
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("clients.get did not yield a Client".to_string()))?;
    let message = tonk_worker_api::LinkAccountRequest {
        message_type: tonk_worker_api::LINK_ACCOUNT.to_string(),
        space: space.to_owned(),
    };
    let message = serde_wasm_bindgen::to_value(&message)
        .map_err(|error| TonkWorkerError::Internal(format!("serialize request: {error}")))?;
    client
        .post_message(&message)
        .map_err(|error| TonkWorkerError::Internal(format!("post_message failed: {error:?}")))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn request_webauthn(
    client: &crate::router::ClientId,
    request: tonk_worker_api::WebAuthnKind,
) -> Result<(), crate::TonkWorkerError> {
    request_webauthn_with(client, request, None, None).await
}

/// [`request_webauthn`], carrying what the worker will do once the page
/// has answered.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn request_webauthn_with(
    client: &crate::router::ClientId,
    request: tonk_worker_api::WebAuthnKind,
    intent: Option<tonk_worker_api::CustodyIntent>,
    credential_id: Option<String>,
) -> Result<(), crate::TonkWorkerError> {
    use crate::TonkWorkerError;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service worker scope".to_string()))?;
    let value = JsFuture::from(global.clients().get(&client.0))
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("clients.get failed: {error:?}")))?;
    if value.is_undefined() || value.is_null() {
        return Err(TonkWorkerError::Conflict(format!(
            "the originating client {} is gone",
            client.0
        )));
    }
    let client: web_sys::Client = value
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("clients.get did not yield a Client".to_string()))?;
    let message = tonk_worker_api::WebAuthnRequest {
        message_type: tonk_worker_api::WEBAUTHN.to_string(),
        request,
        intent,
        credential_id,
    };
    let message = serde_wasm_bindgen::to_value(&message)
        .map_err(|error| TonkWorkerError::Internal(format!("serialize request: {error}")))?;
    // Only a top-level document can run WebAuthn, and only it listens
    // for this. A command asserted from a sealed guest arrives from the
    // guest's own client, so the ask goes to the top-level windows of
    // this origin instead; the one holding the guest is among them, and
    // the relay in each answers at most once.
    if client.frame_type() == web_sys::FrameType::TopLevel {
        return client
            .post_message(&message)
            .map_err(|error| TonkWorkerError::Internal(format!("post_message failed: {error:?}")));
    }
    let options = web_sys::ClientQueryOptions::new();
    options.set_type(web_sys::ClientType::Window);
    let windows = JsFuture::from(global.clients().match_all_with_options(&options))
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("clients.matchAll failed: {error:?}"))
        })?;
    let mut asked = 0;
    for window in js_sys::Array::from(&windows).iter() {
        let Ok(window) = window.dyn_into::<web_sys::Client>() else {
            continue;
        };
        if window.frame_type() != web_sys::FrameType::TopLevel {
            continue;
        }
        if window.post_message(&message).is_ok() {
            asked += 1;
        }
    }
    if asked == 0 {
        return Err(TonkWorkerError::Conflict(
            "no top-level page is open to run the passkey ceremony".into(),
        ));
    }
    Ok(())
}
