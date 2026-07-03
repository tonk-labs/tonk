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

    // The outermost page is a thin SW relay: it registers only the IO-owning
    // `<tonk-host>` and the `<tonk-site>` router, then mounts one `<tonk-site>`.
    // Everything else — the hub, the space chrome, the FAB, the repo content —
    // renders inside `<tonk-site>`'s sealed guests (the `tonk-guest` bundle),
    // which `<tonk-site>` brings up per route. No framework, no per-route
    // components: the profile's `route!` table decides what to render.
    tonk_host::register();
    tonk_portal::register_site();

    // Ensure the default repository + profile exist before routing.
    let _ = tonk_ui::api::init().await;

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    mount_root();
}

/// Mount the single `<tonk-host><tonk-site></tonk-host>` root into `<body>`. The
/// `<tonk-site>` has no `repository`/`branch` ancestor and no `path`, so it
/// routes the document path against the profile branch — the top-level router.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn mount_root() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    // `<tonk-host><tonk-repository profile><tonk-branch meta><tonk-site>`. The
    // `<tonk-repository profile>`/`<tonk-branch meta>` ancestors route the
    // top-level site: `<tonk-site>` reads no context itself, so its `tonk:load`
    // claim and its guest's queries are annotated by these ancestors and reach
    // the profile meta branch — the profile's route! table picks what to render.
    let Ok(host) = document.create_element("tonk-host") else {
        return;
    };
    let Ok(repository) = document.create_element("tonk-repository") else {
        return;
    };
    let _ = repository.set_attribute("profile", "");
    let Ok(branch) = document.create_element("tonk-branch") else {
        return;
    };
    let _ = branch.set_attribute("name", "meta");
    let Ok(site) = document.create_element("tonk-site") else {
        return;
    };
    // `<tonk-site>` never reads `window.location`; the page owns the document
    // path. Set it as `path` and keep it current on navigation (popstate /
    // pushState-driven), so the site re-routes via `attribute_changed_callback`.
    sync_site_path(&site);
    attach_navigation(&site);
    let _ = branch.append_child(&site);
    let _ = repository.append_child(&branch);
    let _ = host.append_child(&repository);
    let _ = body.append_child(&host);
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
