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

use tonk_common::log;

#[cfg(target_arch = "wasm32")]
use crate::detach;

/// On a kernel, its embedder holds the pages: it is told where the page that
/// asked is to go.
#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_navigate(client: Option<&crate::router::ClientId>, href: &str) {
    tell_navigate(client, href, false);
}

/// [`notify_navigate`] as a fresh load replacing the page's history entry.
#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_replace(client: Option<&crate::router::ClientId>, href: &str) {
    tell_navigate(client, href, true);
}

#[cfg(target_arch = "wasm32")]
fn tell_navigate(client: Option<&crate::router::ClientId>, href: &str, replace: bool) {
    let Some(client) = client else {
        log!("navigate: no originating client; skipping redirect");
        return;
    };
    let (client, href) = (client.0.clone(), href.to_owned());
    detach(async move {
        if let Err(error) = tonk_kernel_world::application::navigate(&client, &href, replace).await
        {
            log!("navigate: the embedder was not told: {error:?}");
        }
    });
}

/// Navigation is a page capability, and this host has no page: the
/// target is logged so a host shell (CLI, TUI) that tails the log can
/// still present it. The triggering command has already done its work —
/// only the convenience redirect is absent, mirroring the "client is
/// gone" path above.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn notify_navigate(client: Option<&crate::router::ClientId>, href: &str) {
    let _ = client;
    log!("navigate: no page on this host; the target was {href}");
}

/// No page on this host either; see [`notify_navigate`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn notify_replace(client: Option<&crate::router::ClientId>, href: &str) {
    notify_navigate(client, href);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn notify_profile_changed(_except: Option<&crate::router::ClientId>) {}

/// On a kernel, its embedder holds the pages: it is told to post the reload
/// to every page but `except`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_profile_changed(except: Option<&crate::router::ClientId>) {
    tell_post(
        None,
        except,
        serde_json::json!({ "type": "profile-changed" }),
    );
}

/// Tell the embedder to post `message` to the page `client`, or, with none,
/// to every page but `except`.
#[cfg(target_arch = "wasm32")]
fn tell_post(
    client: Option<&crate::router::ClientId>,
    except: Option<&crate::router::ClientId>,
    message: serde_json::Value,
) {
    let client = client.map(|client| client.0.clone());
    let except = except.map(|client| client.0.clone());
    detach(async move {
        if let Err(error) =
            tonk_kernel_world::application::post(client.as_deref(), except.as_deref(), &message)
                .await
        {
            log!("post: the embedder was not told: {error:?}");
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn notify_profile_changed_to(_client: Option<&crate::router::ClientId>) {}

/// [`notify_profile_changed_to`], told to a kernel's embedder.
#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_profile_changed_to(client: Option<&crate::router::ClientId>) {
    let Some(client) = client else {
        log!("profile change: no originating client; nothing to reload");
        return;
    };
    tell_post(
        Some(client),
        None,
        serde_json::json!({ "type": "profile-changed" }),
    );
}

/// A launch-funnel event with no page to deliver it to — dropped, like
/// the "client is gone" path above. Analytics capture is a browser
/// concern; a native host has nowhere (and no reason) to send it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn notify_analytics(
    client: Option<&crate::router::ClientId>,
    event: tonk_worker_api::AnalyticsEvent,
) {
    let _ = (client, event);
}

/// [`notify_analytics`], told to a kernel's embedder.
#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_analytics(
    client: Option<&crate::router::ClientId>,
    event: tonk_worker_api::AnalyticsEvent,
) {
    let Some(client) = client else {
        log!("analytics: no originating client; skipping event");
        return;
    };
    match serde_json::to_value(tonk_worker_api::AnalyticsMessage::new(event)) {
        Ok(message) => tell_post(Some(client), None, message),
        Err(error) => log!("analytics: failed to serialize event: {error}"),
    }
}

/// A WebAuthn ceremony needs a top-level document, and this host has
/// none: refused rather than stubbed, so callers report the refusal
/// instead of silently succeeding.
pub(crate) async fn request_webauthn_with(
    client: &crate::router::ClientId,
    request: tonk_worker_api::WebAuthnKind,
    intent: Option<tonk_worker_api::CustodyIntent>,
    credential_id: Option<String>,
) -> Result<(), crate::TonkWorkerError> {
    let _ = (client, request, intent, credential_id);
    Err(crate::TonkWorkerError::Conflict(
        "no page is available on this host to run a passkey ceremony".to_string(),
    ))
}

/// Go home: send the page that asked back to the hub.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Home> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::Home) {
        notify_navigate(self.client(), "/");
    }
}
