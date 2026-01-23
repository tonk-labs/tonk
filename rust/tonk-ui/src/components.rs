//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use wasm_bindgen::prelude::*;

use crate::api;

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

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
    /// Service worker is still loading/activating, or setting up upstream.
    Loading,
    /// Service worker is ready and upstream remote is configured.
    Ready,
}

/// The root UI component for the Tonk application.
///
/// This component serves as the main entry point for the Tonk user interface,
/// rendering the primary application view.
///
/// On startup, it waits for the service worker to activate, then automatically
/// sets up the upstream remote if not already configured.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    // Initialize the space: wait for SW, check status, setup remote if needed
    let init_resource = LocalResource::new(|| async {
        log!("Waiting for SW to activate...");
        service_worker_activates().await;
        log!("SW is activated, fetching status...");

        let status = api::status().await?;

        // If no upstream configured, add the remote (but don't set as upstream yet)
        if !status.has_upstream {
            log!("No upstream configured, adding remote...");
            api::authorize().await?;
            log!("Remote added successfully");
        }

        Ok::<_, crate::error::TonkUiError>(())
    });

    // Derive the application status from init resource
    let status = Signal::derive_local(move || {
        match init_resource.get() {
            Some(Ok(())) => Status::Ready,
            Some(Err(e)) => {
                log!("Initialization error: {:?}", e);
                // Still show as loading on error - could add an Error state later
                Status::Loading
            }
            None => Status::Loading,
        }
    });

    provide_context(status);

    view! {
        <TonkLauncher></TonkLauncher>
    }
}
