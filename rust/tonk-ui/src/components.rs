//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use leptos_router::{components::*, path};
use wasm_bindgen::prelude::*;

mod launcher;

mod toolbar;
pub use toolbar::*;

mod space;
pub use space::*;

mod space_redirect;
pub use space_redirect::*;

mod space_router;
pub use space_router::*;

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
    /// Service worker is ready.
    Ready,
}

/// The root UI component for the Tonk application.
///
/// This component serves as the main entry point for the Tonk user interface,
/// rendering the primary application view.
///
/// On startup, it waits for the service worker to activate before rendering
/// the router. This ensures all API requests are intercepted by the SW.
/// Space-specific initialization (like setting up upstream) is handled by the
/// SpaceRouter component which has access to the current space context.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    // Wait for service worker to activate
    let sw_ready = LocalResource::new(|| async {
        log!("Waiting for SW to activate...");
        service_worker_activates().await;
        log!("SW is activated");
        Ok::<_, crate::error::TonkUiError>(())
    });

    // Derive the application status from SW ready state
    let status = Signal::derive_local(move || match sw_ready.get() {
        Some(Ok(())) => Status::Ready,
        Some(Err(e)) => {
            log!("Initialization error: {:?}", e);
            Status::Loading
        }
        None => Status::Loading,
    });

    provide_context(status);

    view! {
        <Suspense fallback=move || view! { <div class="loading">"Initializing..."</div> }>
            {move || sw_ready.get().map(|_| view! {
                <Router>
                    <Routes fallback=|| view! { <div>"Not found"</div> }>
                        // Root path redirects to first space
                        <Route path=path!("") view=SpaceRedirect />
                        // Space-specific routes
                        <Route path=path!(":multikey") view=SpaceRouter />
                        <Route path=path!(":multikey/*any") view=SpaceRouter />
                    </Routes>
                </Router>
            })}
        </Suspense>
    }
}
