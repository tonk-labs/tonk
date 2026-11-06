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
        super::console::trace();
        // Each start of the service worker opens its spaces' sockets
        // anew, so a socket replaced at the same time as this line was
        // replaced by a restart.
        web_sys::console::log_1(&format!("service worker started (build {build_id})").into());
        tonk_worker::set_build_id(build_id);
        tonk_worker::set_asset_paths(asset_paths);
        TonkServiceWorker::new().await
    }
}

/// The console the service worker reports to.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod console {
    use std::io;

    use tracing::Level;
    use tracing_subscriber::filter::Targets;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::prelude::*;

    /// Print dialog's socket reports (each socket opened and closed, and
    /// why, and each watch the service ended) to the console, so a
    /// connection that keeps being replaced says what replaced it.
    pub fn trace() {
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(Console)
            .with_ansi(false)
            .without_time()
            .with_filter(Targets::new().with_target("dialog_remote_ucan", Level::INFO));
        // A second start in the same instance keeps the first subscriber.
        let _ = tracing_subscriber::registry().with(layer).try_init();
    }

    struct Console;

    impl<'a> MakeWriter<'a> for Console {
        type Writer = Line;

        fn make_writer(&'a self) -> Self::Writer {
            Line(Vec::new())
        }
    }

    /// One event, printed once it is written out.
    struct Line(Vec<u8>);

    impl io::Write for Line {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for Line {
        fn drop(&mut self) {
            let line = String::from_utf8_lossy(&self.0);
            web_sys::console::log_1(&line.trim_end().into());
        }
    }
}

#[allow(missing_docs)]
pub fn main() {}
