//! UI binary entrypoint.
//!
//! This binary initializes and mounts the Tonk UI component to the browser DOM.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="ui"` link tag).

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
async fn main() {
    // Panic hook + (when a key is baked in and the user hasn't opted
    // out) posthog init, pageviews, and DOM-event listeners.
    tonk_ui::analytics::install();

    // The outermost page is a thin SW relay: it installs the IO-owning host
    // (document-level listeners — no element) and the `<tonk-site>` router,
    // then mounts one `<tonk-site>`. Everything else — the hub, the space
    // chrome, the FAB, the repo content — renders inside `<tonk-site>`'s
    // sealed guests (the `tonk-guest` bundle), which `<tonk-site>` brings up
    // per route. No framework, no per-route components: the profile's
    // `route!` table decides what to render.
    tonk_portal::register_site();

    // Ensure the default repository + profile exist before routing.
    let _ = tonk_ui::api::init().await;

    // Install the host AFTER init: init awaits service-worker readiness, so
    // everything the host wires up runs against a controlling SW. Nothing
    // dispatches consumer events before `mount_root` below, so the late
    // install loses no operations. (Sync cadence is SW-owned — the page
    // runs no heartbeat.)
    tonk_host::install();

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    mount_root();
}

/// Mount the single `<tonk-site>` root into `<body>` — the top-level router.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn mount_root() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    // `<tonk-site with="main@profile:tonk" allow="*">`: `with` routes the site's
    // `tonk:load` claim and its guest's queries at the profile's main branch —
    // the profile's route! table picks what to render. `allow="*"` makes this
    // the privileged site: its guest (the trusted profile chrome) may reach
    // into any space (hub cards, sync chips, the FAB space list).
    let Ok(site) = document.create_element("tonk-site") else {
        return;
    };
    let _ = site.set_attribute("with", "main@profile:tonk");
    let _ = site.set_attribute("allow", "*");
    // `<tonk-site>` never reads `window.location`; the page owns the document
    // path. Set it as `path` and keep it current on navigation (popstate /
    // pushState-driven), so the site re-routes via `attribute_changed_callback`.
    sync_site_path(&site);
    attach_navigation(&site);
    let _ = body.append_child(&site);
}

/// Set the top-level `<tonk-site>`'s `path` attribute to the document path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn sync_site_path(site: &web_sys::Element) {
    let path = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_owned());
    let _ = site.set_attribute("path", &path);
}

/// Keep the top-level site's `path` in sync with the URL: re-set it on
/// `popstate` (back/forward and the `navigate` command's pushState + popstate).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn attach_navigation(site: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let Some(win) = web_sys::window() else {
        return;
    };
    let site = site.clone();
    let on_popstate = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        sync_site_path(&site);
    });
    let _ = win.add_event_listener_with_callback("popstate", on_popstate.as_ref().unchecked_ref());
    on_popstate.forget();
}

/// Append `<script type="module" src="/hot-swap.js">` to the document
/// head. Debug-only — see the call site.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", debug_assertions))]
fn inject_hot_swap() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(head) = document.head() else {
        return;
    };
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("type", "module");
    let _ = script.set_attribute("src", "/hot-swap.js");
    let _ = head.append_child(&script);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
