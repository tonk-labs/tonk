//! Redirecting the page that asked — the worker side of a navigation.
//!
//! A command handler whose effect is a page capability (loading a new
//! location) can't perform it itself: the service worker has no
//! `window`. It asks through state instead of a message: the desired
//! location is asserted as [`SiteTarget`](tonk_schema::SiteTarget) on the
//! tab's site entity, in the state layer of the branch the site is
//! stamped on. The tab's `<tonk-site>` already subscribes to its own
//! stamp; it sees the target, navigates, and the `tonk:load` it then
//! fires re-stamps the site, which clears the target. Used by the join
//! handler (redirect into the joined space) and the create handler (drop
//! the creator into the fresh space).

use tonk_common::log;

/// Ask the originating client's tab to go to `href`.
///
/// The client's sites come from the liveness ledger; each is written a
/// [`SiteTarget`](tonk_schema::SiteTarget) on the branch whose state layer
/// holds the site's stamp, and that branch is scheduled for a poll so the
/// tab's subscription delivers the target. The caller drains the scheduled
/// polls, as after any state write.
///
/// Returns `false`, with a log, when nothing was written: the client is
/// unknown, has stamped no site, or no branch holds its stamp. The
/// triggering command still succeeded; only the convenience redirect is
/// lost, and the user can navigate from the Hub.
pub(crate) async fn request_navigation(
    tonk: &crate::worker::TonkState,
    client: Option<&crate::router::ClientId>,
    href: &str,
) -> bool {
    let Some(client) = client else {
        log!("navigate: no originating client; skipping redirect to {href}");
        return false;
    };
    let sites: Vec<dialog_artifacts::Entity> = tonk
        .clients
        .read()
        .await
        .get(client)
        .map(|state| state.sites.iter().filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_default();
    if sites.is_empty() {
        log!(
            "navigate: client {} has stamped no site; skipping redirect to {href}",
            client.0
        );
        return false;
    }
    let mut written = false;
    for branch in tonk.reactor.cached_branch_states() {
        for site in &sites {
            let stamped = !branch
                .state_layer()
                .scan(&dialog_artifacts::ArtifactSelector::new().of(site.clone()))
                .is_empty();
            if !stamped {
                continue;
            }
            match branch
                .write(
                    tonk_schema::SiteTarget::new(site.clone(), href),
                    &tonk.operator,
                )
                .await
            {
                Ok(()) => {
                    written = true;
                    tonk.reactor.schedule_poll(std::sync::Arc::clone(&branch));
                }
                Err(error) => log!("navigate: target write for {site} failed: {error}"),
            }
        }
    }
    if !written {
        log!(
            "navigate: no branch holds a site of client {}; skipping redirect to {href}",
            client.0
        );
    }
    written
}

/// Ask every other top-level document to reload after the active browser
/// profile changes. The message carries no profile or account identifier.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn notify_profile_changed(except: Option<&crate::router::ClientId>) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    let except = except.map(|client| client.0.clone());
    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(global) => global,
        Err(_) => {
            log!("profile change: not in a service worker scope; skipping reload broadcast");
            return;
        }
    };
    spawn_local(async move {
        let options = web_sys::ClientQueryOptions::new();
        options.set_type(web_sys::ClientType::Window);
        let windows = match JsFuture::from(global.clients().match_all_with_options(&options)).await
        {
            Ok(windows) => windows,
            Err(error) => {
                log!("profile change: clients.matchAll failed: {error:?}");
                return;
            }
        };
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &JsValue::from_str("type"),
            &JsValue::from_str("profile-changed"),
        );
        for value in js_sys::Array::from(&windows).iter() {
            let Ok(client) = value.dyn_into::<web_sys::Client>() else {
                continue;
            };
            if client.frame_type() != web_sys::FrameType::TopLevel
                || except.as_deref() == Some(client.id().as_str())
            {
                continue;
            }
            if let Err(error) = client.post_message(&message) {
                log!("profile change: reload message failed: {error:?}");
            }
        }
    });
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn notify_profile_changed(_except: Option<&crate::router::ClientId>) {}

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

/// A launch-funnel event with no page to deliver it to — dropped, like
/// the "client is gone" path above. Analytics capture is a browser
/// concern; a native host has nowhere (and no reason) to send it.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn notify_analytics(
    client: Option<&crate::router::ClientId>,
    event: tonk_worker_api::AnalyticsEvent,
) {
    let _ = (client, event);
}

/// Ask the originating document to run a WebAuthn ceremony the worker
/// cannot: it has no `window`. The page answers through the ordinary API
/// (`POST /api/identity/root`), which is what the worker then waits on.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
/// Ask the page to add an account, on behalf of `space`.
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

/// No page exists on this host to show a registration UI, so the ask is
/// refused up front — the same outcome as the browser's "originating
/// client is gone", and the caller's refusal reporting carries it to
/// whoever asked.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn request_account_link<'a>(
    client: &'a crate::router::ClientId,
    space: &'a str,
) -> Result<(), crate::TonkWorkerError> {
    let _ = (client, space);
    Err(crate::TonkWorkerError::Conflict(
        "no page is available on this host to add an account".to_string(),
    ))
}

/// A WebAuthn ceremony needs a top-level document, and this host has
/// none — refused rather than stubbed, so callers report the refusal
/// (via `ceremony::report` and friends) instead of silently succeeding.
/// A host that grows its own credential ceremony replaces this seam.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn request_webauthn(
    client: &crate::router::ClientId,
    request: tonk_worker_api::WebAuthnKind,
) -> Result<(), crate::TonkWorkerError> {
    request_webauthn_with(client, request, None, None).await
}

/// [`request_webauthn`], carrying what the worker would do once a page
/// answered — see the native `request_webauthn` above for why this is a
/// refusal.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::request_navigation;
    use crate::router::ClientId;

    /// With no originating client, or one that stamped no site, there is
    /// no tab to move and nothing is written anywhere.
    #[dialog_common::test]
    async fn it_skips_a_redirect_without_a_stamped_site() {
        let tonk = crate::router::tests::test_state().await;
        assert!(!request_navigation(&tonk, None, "/space/x").await);
        let unknown = ClientId("never-seen".into());
        assert!(!request_navigation(&tonk, Some(&unknown), "/space/x").await);
    }

    /// A client whose site is stamped on a branch gets the target written
    /// on that site, in that branch's state layer, and nowhere else.
    #[dialog_common::test]
    async fn it_writes_the_target_on_the_clients_stamped_site() {
        use dialog_artifacts::ArtifactSelector;

        let tonk = crate::router::tests::test_state().await;
        let client = ClientId("tab-1".into());
        let site: dialog_artifacts::Entity = "site:tab-1".parse().unwrap();
        let main = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let mut stamp = dialog_artifacts::Changes::new();
        dialog_query::Statement::assert(
            dialog_query::the!("xyz.tonk.site/path")
                .of(site.clone())
                .is("/join".to_string()),
            &mut stamp,
        );
        main.state.write(stamp, &tonk.operator).await.unwrap();
        tonk.clients
            .write()
            .await
            .entry(client.clone())
            .or_default()
            .sites
            .insert(site.to_string());

        assert!(request_navigation(&tonk, Some(&client), "/space/x").await);

        let target = main.state.state_layer().scan(
            &ArtifactSelector::new()
                .of(site.clone())
                .the("xyz.tonk.site/target".parse().unwrap()),
        );
        assert_eq!(target.len(), 1);
        assert_eq!(
            target[0].is,
            dialog_artifacts::Value::String("/space/x".into())
        );
    }
}
