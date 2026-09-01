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
//! - the worker's `tonk:local-commit` BroadcastChannel → `commit`.
//!   Not the `tonk:committed` window event: its dispatchers live in
//!   sealed guest iframes whose window events never reach this page,
//!   while a BroadcastChannel crosses that boundary (and tabs — see
//!   the visibility gate at the listener);
//! - `activate` → `sheet_activated` (bubbles from `<tonk-sheet-binder>`);
//! - `tonk:analytics` → generic channel: any element may dispatch
//!   `CustomEvent("tonk:analytics", { bubbles: true, composed: true,
//!   detail: { name, props } })` without depending on this crate.

// The wasm-bindgen executor needs no initialization: `install()` runs from
// the ui binary's `main`, which mounts custom elements rather than starting a
// framework runtime, so there is no global executor to spawn onto.
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

/// Console panic reporting (always) plus a content-free `panic` event. The
/// local console retains the diagnostic; analytics gets only a static type and
/// repository-relative source location.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let location = info
            .location()
            .map(|location| {
                let file = location.file();
                let file = file
                    .find("rust/")
                    .map(|index| &file[index..])
                    .unwrap_or("unknown");
                format!("{file}:{}", location.line())
            })
            .unwrap_or_else(|| "unknown".to_owned());
        let fingerprint = format!("wasm_panic:{location}");
        tonk_analytics::web::capture(
            tonk_analytics::event::PANIC,
            &serde_json::json!({
                "type": "wasm_panic",
                "location": location,
                "fingerprint": fingerprint,
            }),
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
    let _ =
        window.add_event_listener_with_callback("popstate", on_popstate.as_ref().unchecked_ref());
    on_popstate.forget();

    // Local commits: the worker announces every durable transact /
    // evaluate commit on this fixed channel (tonk-worker's
    // `broadcast::LOCAL_COMMIT_CHANNEL`). BroadcastChannel reaches
    // every tab of the origin, so only the visible tab captures —
    // otherwise N open tabs would each record the same commit.
    if let Ok(channel) = web_sys::BroadcastChannel::new("tonk:local-commit") {
        let on_commit = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                let visible = web_sys::window()
                    .and_then(|w| w.document())
                    .map(|d| d.visibility_state() == web_sys::VisibilityState::Visible)
                    .unwrap_or(true);
                if !visible {
                    return;
                }
                // The payload's branch is capped to a closed vocabulary
                // so a future user-named branch can't leak.
                let branch = event
                    .data()
                    .as_string()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .and_then(|value| {
                        value
                            .get("branch")
                            .and_then(|b| b.as_str())
                            .map(str::to_owned)
                    })
                    .map(|b| match b.as_str() {
                        "main" | "meta" => b,
                        _ => "other".to_owned(),
                    })
                    .unwrap_or_else(|| "unknown".to_owned());
                tonk_analytics::web::capture(
                    tonk_analytics::event::COMMIT,
                    &serde_json::json!({ "branch": branch }),
                );
            },
        );
        channel.set_onmessage(Some(on_commit.as_ref().unchecked_ref()));
        on_commit.forget();
        // The channel handle must stay alive for the subscription to
        // keep firing; it lives for the page like the listeners above.
        std::mem::forget(channel);
    }

    let on_activate = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        tonk_analytics::web::capture(
            tonk_analytics::event::SHEET_ACTIVATED,
            &serde_json::json!({}),
        );
    });
    let _ =
        window.add_event_listener_with_callback("activate", on_activate.as_ref().unchecked_ref());
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
