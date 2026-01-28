//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use tonk_worker::AuthorizeRequest;
use wasm_bindgen::prelude::*;

use crate::api;

mod auth;
use auth::*;

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

mod topbar;
use topbar::*;

mod space;
use space::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = serviceWorkerActivates)]
    async fn service_worker_activates();
}

/// The current status of the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Service worker is still loading/activating.
    Loading,
    /// Service worker is ready but no upstream remote is configured.
    Unauthorized,
    /// Authorization request has been sent, waiting for response.
    Authorizing,
    /// Service worker is ready and upstream remote is configured.
    Authorized,
}

/// The root UI component for the Tonk application.
///
/// This component serves as the main entry point for the Tonk user interface,
/// rendering the primary application view.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    // Fetch status after service worker is ready
    let status_resource = LocalResource::new(|| async {
        log!("Waiting for SW to activate...");
        service_worker_activates().await;
        log!("SW is activated, fetching status...");
        api::status().await
    });

    let authorize_action =
        Action::new_local(|request: &AuthorizeRequest| api::authorize(request.clone()));

    // Derive the application status from status resource and authorize action
    let status = Signal::derive_local(move || {
        // Check if status has loaded
        let response = match status_resource.get() {
            Some(Ok(response)) => response,
            _ => return Status::Loading,
        };

        // Check if already authorized (has upstream)
        if response.has_upstream {
            return Status::Authorized;
        }

        // Check if authorization is in progress
        if authorize_action.pending().get() {
            return Status::Authorizing;
        }

        // Check if authorization succeeded
        if let Some(Ok(response)) = authorize_action.value().get()
            && response.success
        {
            return Status::Authorized;
        }

        Status::Unauthorized
    });

    provide_context(status);
    provide_context(authorize_action);

    view! {
        <TonkAuth></TonkAuth>
        <TonkLauncher></TonkLauncher>
    }
}
