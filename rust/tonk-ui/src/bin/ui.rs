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

    // Install the host IO surface and mount `<tonk-site>` right away —
    // first paint no longer waits on any data round-trip. Every `/api/*`
    // fetch the host issues self-gates on service-worker readiness
    // (`tonk_host::ready::wait`, memoized), so mounting before the SW is
    // controlling loses nothing: the site's own routing fetches block
    // themselves until the worker is up.
    tonk_host::install();

    // Passkey ceremonies live on the window: `navigator.credentials`
    // does not exist in the service worker, and each ceremony needs a
    // user gesture. The worker never sees root-key material. The hook
    // installs as `window.tonkIdentity`, deliberately outside
    // `window.tonk` — tonk-host's page-effect forwarding uses the bare
    // presence of `window.tonk` to detect a portal guest, and the top
    // page must never look like one.
    tonk_identity::install();
    tonk_ui::identity_gate::install();
    tonk_ui::account::register();

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    mount_root();
}

/// Mount the top-document shell. Account routes bypass sealed guests because
/// WebAuthn ceremonies must run in the RP ID's top-level origin.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn mount_root() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let Ok(shell) = document.create_element("div") else {
        return;
    };
    let _ = shell.set_attribute("id", "tonk-root");
    render_root(&shell);
    attach_navigation(&shell);
    let _ = body.append_child(&shell);
}

/// Render or update the correct top-document root for the current path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn render_root(shell: &web_sys::Element) {
    let path = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_owned());
    let account_route = path == "/account" || path.starts_with("/account/");
    let current = shell.first_element_child();

    if account_route {
        if current.as_ref().map(web_sys::Element::tag_name).as_deref() != Some("TONK-ACCOUNT") {
            shell.set_inner_html("");
            if let Some(document) = shell.owner_document()
                && let Ok(account) = document.create_element("tonk-account")
            {
                let _ = shell.append_child(&account);
            }
        }
        return;
    }

    if let Some(site) = current.filter(|element| element.tag_name() == "TONK-SITE") {
        let _ = site.set_attribute("path", &path);
        return;
    }

    shell.set_inner_html("");
    let Some(document) = shell.owner_document() else {
        return;
    };
    let Ok(site) = document.create_element("tonk-site") else {
        return;
    };
    let _ = site.set_attribute("with", "main@profile:tonk");
    let _ = site.set_attribute("allow", "*");
    let _ = site.set_attribute("path", &path);
    let _ = shell.append_child(&site);
}

/// Keep the top-document root in sync with client-side navigation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn attach_navigation(shell: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let Some(win) = web_sys::window() else {
        return;
    };
    let shell = shell.clone();
    let on_popstate = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
        render_root(&shell);
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
