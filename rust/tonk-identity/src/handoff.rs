//! Liveness for the page/worker custody message channel.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/custody-channel.mjs")]
extern "C" {
    #[wasm_bindgen(js_name = waitForCustodyReply)]
    pub(crate) fn wait_for_custody_reply(
        port: web_sys::MessagePort,
        timeout_ms: i32,
    ) -> js_sys::Promise;

    #[wasm_bindgen(catch, js_name = startCustodyProgress)]
    fn start_custody_progress(port: &web_sys::MessagePort) -> Result<js_sys::Function, JsValue>;
}

/// Keep a custody reply listener alive only while its worker handler is alive.
/// The caller must retain this guard inside the message event's lifetime.
pub struct CustodyProgress(js_sys::Function);

impl CustodyProgress {
    /// Start progress for a page that opted into the progress protocol.
    pub fn start(port: &web_sys::MessagePort) -> Result<Self, JsValue> {
        start_custody_progress(port).map(Self)
    }
}

impl Drop for CustodyProgress {
    fn drop(&mut self) {
        let _ = self.0.call0(&JsValue::UNDEFINED);
    }
}
