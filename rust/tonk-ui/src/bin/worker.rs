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
    ///
    /// `build_id` is the identity `scripts/stamp-service-worker.sh` stamps into
    /// `service_worker.js`. It names the per-build caches, and the JS
    /// shim passes it in rather than the two sides hardcoding the same
    /// literal — a name kept in step by hand across two languages is
    /// drift waiting to happen, and the drift is silent (one side
    /// purges caches the other is still writing).
    #[wasm_bindgen]
    pub async fn activate(
        build_id: String,
        asset_paths: JsValue,
    ) -> Result<TonkServiceWorker, JsError> {
        let asset_paths: Vec<String> = serde_wasm_bindgen::from_value(asset_paths)
            .map_err(|error| JsError::new(&format!("invalid stamped asset paths: {error}")))?;
        tonk_worker::set_build_id(build_id);
        tonk_worker::set_asset_paths(asset_paths);
        TonkServiceWorker::new().await
    }
}

#[allow(missing_docs)]
pub fn main() {}
