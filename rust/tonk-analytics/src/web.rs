//! Browser-side capture: a thin bridge over the self-hosted
//! posthog-js bundle that `tonk-ui`'s `index.html` loads. Every entry
//! point is a guarded no-op until [`init`] succeeds, and `init` fails
//! closed: no key, no bundle, or a localStorage opt-out all leave
//! capture disabled with zero network activity.

use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::prelude::*;

static ENABLED: AtomicBool = AtomicBool::new(false);

#[wasm_bindgen(inline_js = r#"
export function ph_init(key, host) {
    try {
        if (!key || !window.posthog) return false;
        if (localStorage.getItem("tonk:telemetry") === "off") return false;
        window.posthog.init(key, {
            api_host: host,
            autocapture: false,
            capture_pageview: false,
            disable_session_recording: true,
            persistence: "memory",
            person_profiles: "identified_only",
        });
        return true;
    } catch (e) {
        return false;
    }
}
export function ph_capture(name, props_json) {
    try { window.posthog.capture(name, JSON.parse(props_json)); } catch (e) {}
}
export function ph_identify(id) {
    try { window.posthog.identify(id); } catch (e) {}
}
"#)]
extern "C" {
    fn ph_init(key: &str, host: &str) -> bool;
    fn ph_capture(name: &str, props_json: &str);
    fn ph_identify(id: &str);
}

/// Initialize posthog-js. Returns whether capture is live; when it
/// returns `false` every other function in this module stays inert.
pub fn init() -> bool {
    let Some(key) = crate::api_key() else {
        ENABLED.store(false, Ordering::Relaxed);
        return false;
    };
    let live = ph_init(&key, &crate::host());
    ENABLED.store(live, Ordering::Relaxed);
    live
}

/// Forward one event with a JSON-object `properties` value.
pub fn capture(name: &str, properties: &serde_json::Value) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    ph_capture(name, &properties.to_string());
}

/// Capture a `$pageview` for `path`, normalized so no space/entity
/// names leave the machine (see [`crate::normalize_path`]).
pub fn capture_pageview(path: &str) {
    let route = crate::normalize_path(path);
    capture(
        crate::event::PAGEVIEW,
        &serde_json::json!({ "$current_url": route, "route": route }),
    );
}

/// Tie this device to the hashed profile identity
/// (see [`crate::distinct_id`] — pass its output, never a raw DID).
pub fn identify(distinct_id: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    ph_identify(distinct_id);
}
