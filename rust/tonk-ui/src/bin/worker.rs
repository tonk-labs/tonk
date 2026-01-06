//! Service worker activation binary.
//!
//! This binary provides the activation handler for the Tonk service worker.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="worker"` link tag).

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod main {
    use tonk_worker::TonkServiceWorker;
    use wasm_bindgen::prelude::*;

    /// Activates and initializes the Tonk service worker.
    #[wasm_bindgen]
    pub async fn activate() -> Result<TonkServiceWorker, JsError> {
        TonkServiceWorker::new().await
    }
}

#[allow(missing_docs)]
pub fn main() {}
