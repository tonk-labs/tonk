//! PostHog wiring for the shell page.
//!
//! One `install()` call from the ui binary: sets the panic hook, and
//! — only when `tonk_analytics::web::init()` reports capture is live —
//! records `app_loaded`, the initial pageview, and attaches window
//! listeners that translate the page's existing DOM events into
//! analytics events:
//!
//! - `popstate` → `$pageview` (the `navigate` command pushes state
//!   then fires `popstate`, so this sees every route change);
//! - `tonk:committed` → `commit` (dispatched by the workspace sync
//!   elements, the inspector, and the worker's join flow);
//! - `activate` → `sheet_activated` (bubbles from `<tonk-sheet-binder>`);
//! - `tonk:analytics` → generic channel: any element may dispatch
//!   `CustomEvent("tonk:analytics", { bubbles: true, composed: true,
//!   detail: { name, props } })` without depending on this crate.

// Not `leptos::task::spawn_local`: `install()` runs from the ui
// binary's `main`, where no Leptos global executor is initialized
// (the shell mounts custom elements, not Leptos components), and
// any_spawner panics on an uninitialized executor. The wasm-bindgen
// executor needs no initialization.
use wasm_bindgen_futures::spawn_local;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::api;

/// Install the panic hook and, when analytics is enabled, boot
/// capture. Call exactly once, before mounting the shell.
pub fn install() {
    install_panic_hook();
    if !tonk_analytics::web::init() {
        return;
    }
    tonk_analytics::web::capture(
        tonk_analytics::event::APP_LOADED,
        &serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
    );
    capture_current_pageview();
    attach_listeners();
    spawn_local(identify());
}

/// Console panic reporting (always) plus a content-free `panic` event
/// (first line of the panic message only) when capture is live.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let message = info
            .to_string()
            .lines()
            .next()
            .unwrap_or("panic")
            .to_owned();
        tonk_analytics::web::capture(
            tonk_analytics::event::PANIC,
            &serde_json::json!({ "message": message }),
        );
    }));
}

fn capture_current_pageview() {
    let path = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_else(|| "/".to_owned());
    tonk_analytics::web::capture_pageview(&path);
}

fn attach_listeners() {
    let Some(window) = web_sys::window() else {
        return;
    };

    let on_popstate =
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_| capture_current_pageview());
    let _ = window
        .add_event_listener_with_callback("popstate", on_popstate.as_ref().unchecked_ref());
    on_popstate.forget();

    let on_commit = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        tonk_analytics::web::capture(tonk_analytics::event::COMMIT, &serde_json::json!({}));
    });
    let _ = window
        .add_event_listener_with_callback("tonk:committed", on_commit.as_ref().unchecked_ref());
    on_commit.forget();

    let on_activate = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        tonk_analytics::web::capture(
            tonk_analytics::event::SHEET_ACTIVATED,
            &serde_json::json!({}),
        );
    });
    let _ = window
        .add_event_listener_with_callback("activate", on_activate.as_ref().unchecked_ref());
    on_activate.forget();

    // Dispatchers own the privacy contract: `detail.props` is
    // forwarded verbatim, so payloads must stay content-free
    // (hashes and counts only) — see docs/telemetry.md.
    let on_custom = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(custom) = event.dyn_ref::<web_sys::CustomEvent>() else {
            return;
        };
        let detail = custom.detail();
        let Some(name) = js_sys::Reflect::get(&detail, &"name".into())
            .ok()
            .and_then(|value| value.as_string())
        else {
            return;
        };
        let props = js_sys::Reflect::get(&detail, &"props".into())
            .ok()
            .and_then(|value| js_sys::JSON::stringify(&value).ok())
            .map(String::from)
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        tonk_analytics::web::capture(&name, &props);
    });
    let _ = window
        .add_event_listener_with_callback("tonk:analytics", on_custom.as_ref().unchecked_ref());
    on_custom.forget();
}

/// Fetch the profile DID from the worker and identify with its hash,
/// so web and CLI activity from one profile correlate. Best-effort.
async fn identify() {
    if let Ok(response) = api::identify().await {
        tonk_analytics::web::identify(&tonk_analytics::distinct_id(&response.did));
    }
}
