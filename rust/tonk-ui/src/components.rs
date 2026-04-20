//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use wasm_bindgen::prelude::*;

mod empty;
use empty::*;

mod join;
pub use join::*;

mod launcher;
use launcher::*;

mod modal;
pub use modal::*;

mod repo;
use repo::*;

mod sidebar;
pub use sidebar::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = serviceWorkerActivates)]
    async fn service_worker_activates();

    /// Triggers a sync operation.
    /// Uses Background Sync API if available, otherwise falls back to /api/sync.
    #[wasm_bindgen(js_namespace = window, catch)]
    pub async fn sync() -> Result<(), JsValue>;
}

/// The root UI component.
///
/// Waits for the service worker to take control before rendering, so any
/// `fetch` issued by the routed view lands on the worker rather than
/// falling through to the SPA index. Rendering blocks on SW activation
/// rather than showing a loading state: the delay is typically a single
/// tick, and a flashing skeleton produces worse UX than a brief blank.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    let ready = LocalResource::new(|| async {
        service_worker_activates().await;
    });

    view! {
        {move || ready.get().map(|_| view! { <TonkLauncher /> })}
    }
}
