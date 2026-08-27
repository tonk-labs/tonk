//! UI binary entrypoint.
//!
//! This binary initializes and mounts the Tonk UI component to the browser DOM.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="ui"` link tag).

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn canonical_account_url(path: &str, search: &str) -> Option<String> {
    if path == "/settings" || path.starts_with("/settings/") {
        return Some(format!("{path}{search}"));
    }
    if path == "/account" {
        return Some(format!("/settings{search}"));
    }
    path.strip_prefix("/account/")
        .map(|suffix| format!("/settings/{suffix}{search}"))
}

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

    // Install the host IO surface before awaiting readiness; it registers
    // document-level hooks but does not mount application elements. The
    // top-document root waits below for the strict service-worker gate, while
    // every later `/api/*` fetch retains the tolerant memoized host gate.
    tonk_host::install();

    // Passkey ceremonies live on the window: `navigator.credentials`
    // does not exist in the service worker, and each ceremony needs a
    // user gesture. The worker never sees root-key material. The hook
    // installs as `window.tonkIdentity`, deliberately outside
    // `window.tonk` — tonk-host's page-effect forwarding uses the bare
    // presence of `window.tonk` to detect a portal guest, and the top
    // page must never look like one.
    tonk_identity::install();
    tonk_ui::custody_relay::install();
    // A guest asking to register raises the dialog here, in the only
    // document that can run the ceremony.
    tonk_portal::on_register(|reason, return_focus| {
        match return_focus {
            Some(return_focus) => tonk_ui::register_dialog::open_with_return_focus(move || {
                return_focus.restore();
            }),
            None => tonk_ui::register_dialog::open(),
        }
        tonk_ui::register_dialog::describe(reason);
    });
    tonk_ui::account::register();
    tonk_ui::activate::register();

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    if let Err(error) = tonk_host::ready::require().await {
        web_sys::console::error_1(&error);
        return;
    }
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
    let Some(window) = web_sys::window() else {
        return;
    };
    let path = window
        .location()
        .pathname()
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_owned());
    let search = window.location().search().unwrap_or_default();
    let canonical_account = canonical_account_url(&path, &search);
    if path == "/account" || path.starts_with("/account/") {
        if let Some(canonical) = canonical_account {
            let _ = window.location().replace(&canonical);
        }
        return;
    }
    let account_route = canonical_account.is_some();
    let activate_route = path == "/activate" || path.starts_with("/activate/");
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

    // The activation email lands here on any device, signed in or not,
    // so the page bypasses sealed guests exactly as the account page
    // does.
    if activate_route {
        if current.as_ref().map(web_sys::Element::tag_name).as_deref() != Some("TONK-ACTIVATE") {
            shell.set_inner_html("");
            if let Some(document) = shell.owner_document()
                && let Ok(activate) = document.create_element("tonk-activate")
            {
                let _ = shell.append_child(&activate);
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

#[cfg(test)]
mod tests {
    use super::canonical_account_url;

    #[test]
    fn it_canonicalizes_settings_routes_without_losing_query_parameters() {
        assert_eq!(
            canonical_account_url("/settings", "?next=%2Fspace%2Fone"),
            Some("/settings?next=%2Fspace%2Fone".into())
        );
        assert_eq!(
            canonical_account_url("/settings/link", "?callback=http%3A%2F%2Flocalhost"),
            Some("/settings/link?callback=http%3A%2F%2Flocalhost".into())
        );
        assert_eq!(
            canonical_account_url("/account", "?revoke=did%3Akey%3Aone"),
            Some("/settings?revoke=did%3Akey%3Aone".into())
        );
        assert_eq!(
            canonical_account_url("/account/link", "?audience=did%3Akey%3Acli"),
            Some("/settings/link?audience=did%3Akey%3Acli".into())
        );
        assert_eq!(canonical_account_url("/space/one", ""), None);
    }
}
