//! Browser-side capture: a thin bridge over the self-hosted
//! posthog-js bundle that `tonk-ui`'s `index.html` loads. Every entry
//! point is a guarded no-op until [`init`] succeeds, and `init` fails
//! closed: no key, no bundle, or a localStorage opt-out all leave
//! capture disabled with zero network activity.

use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::prelude::*;

static ENABLED: AtomicBool = AtomicBool::new(false);

#[wasm_bindgen(inline_js = r#"
export function ph_init(key, host, version) {
    try {
        if (!key || !window.posthog) return false;
        if (localStorage.getItem("tonk:telemetry") === "off") return false;
        window.posthog.init(key, {
            api_host: host,
            autocapture: false,
            capture_pageview: false,
            disable_session_recording: true,
            capture_performance: false,
            capture_dead_clicks: false,
            capture_heatmaps: false,
            capture_exceptions: false,
            persistence: "memory",
            person_profiles: "identified_only",
            // posthog-js enriches every event (and the $set/$set_once
            // person-property blocks, which $identify carries at the
            // event's top level) with the real page URL and referrer;
            // the app's routes embed space/entity names, so the
            // normalize_path guarantee is re-enforced here: drop the
            // raw-URL fields and let the pre-normalized `route`
            // property stand in for $current_url.
            before_send: function (payload) {
                if (!payload) return payload;
                var strip = function (obj) {
                    if (!obj) return;
                    delete obj.$pathname;
                    delete obj.$referrer;
                    delete obj.$referring_domain;
                    for (var key in obj) {
                        if (key.indexOf("$initial_") === 0) delete obj[key];
                    }
                };
                strip(payload.$set);
                strip(payload.$set_once);
                var props = payload.properties;
                if (props) {
                    strip(props);
                    strip(props.$set);
                    strip(props.$set_once);
                    if (props.route) {
                        props.$current_url = props.route;
                    } else {
                        delete props.$current_url;
                    }
                }
                return payload;
            },
        });
        // Super property on every event: which deployment this is.
        // Dashboards filter/break down on it to keep production,
        // staging, dev, and the marketing site (which registers
        // environment = "website" in its own repo) apart within the
        // shared PostHog project.
        // tonk.network is the production origin the CLI builds invite links
        // against. Staging has only its staging.tonk.xyz origin.
        var host = location.hostname;
        var environment =
            host === "tonk.network" ? "production"
            : host === "staging.tonk.xyz" ? "staging"
            : "dev";
        window.posthog.register({
            environment: environment,
            version: version
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
    fn ph_init(key: &str, host: &str, version: &str) -> bool;
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
    let live = ph_init(&key, &crate::host(), env!("CARGO_PKG_VERSION"));
    ENABLED.store(live, Ordering::Relaxed);
    live
}

/// Forward one event with a JSON-object `properties` value.
pub fn capture(name: &str, properties: &serde_json::Value) {
    // Account events may only enter through `capture_account`, whose closed
    // serializer rejects malformed shapes. This also prevents the generic DOM
    // event bridge from impersonating the account taxonomy.
    if name == crate::event::ACCOUNT {
        return;
    }
    capture_unchecked(name, properties);
}

fn capture_unchecked(name: &str, properties: &serde_json::Value) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    ph_capture(name, &properties.to_string());
}

/// Validate and capture one canonical account event.
pub fn capture_account(
    event: &crate::account::AccountEvent,
) -> Result<(), crate::account::ValidationError> {
    let properties = event.validated_properties()?;
    capture_unchecked(
        crate::event::ACCOUNT,
        &serde_json::Value::Object(properties),
    );
    Ok(())
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
