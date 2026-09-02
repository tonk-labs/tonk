//! UI binary entrypoint.
//!
//! This binary initializes and mounts the Tonk UI component to the browser DOM.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="ui"` link tag).

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn canonical_account_url(path: &str, search: &str) -> Option<String> {
    if path == "/account" || path.starts_with("/account/") {
        return Some(format!("{path}{search}"));
    }
    // `/settings` belongs to the hub now (a real route: the settings page
    // per the wireframes). The account panel — the WebAuthn and
    // destructive ceremonies — lives at `/account`; old deep links
    // (`/settings/link`, `/settings?add=1`) redirect to it, while the bare
    // path falls through to the routed page.
    if path == "/settings" && search.is_empty() {
        return None;
    }
    if path == "/settings" {
        return Some(format!("/account{search}"));
    }
    path.strip_prefix("/settings/")
        .map(|suffix| format!("/account/{suffix}{search}"))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::{JsCast, prelude::*};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const READINESS_FAILURE_MESSAGE: &str =
    "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.";

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
        // The guest steers the anchored ceremony from the OTHER side of
        // the frame boundary — it cannot reach the top-page cluster, so
        // it asks. Tab switches SUSPEND and SHOW the cluster (a tab bar
        // hides the background tab's content, it does not destroy it);
        // dismiss remains the true teardown.
        let request = tonk_ui::register_dialog::parse_request(reason);
        match request.reason.as_str() {
            "dismiss" => {
                tonk_ui::register_dialog::close();
                return;
            }
            "suspend" => {
                tonk_ui::register_dialog::suspend();
                return;
            }
            "show" => {
                tonk_ui::register_dialog::resume();
                return;
            }
            _ => {}
        }
        if tonk_ui::register_dialog::is_open() {
            // A repeat request re-shows the standing cluster — everything
            // typed survives the round trip through the spaces tab.
            tonk_ui::register_dialog::resume();
            return;
        }
        if request.anchor.is_none() && !request.space.is_empty() {
            // A blocked share: the linking screen IS the hub's settings
            // route. The space rides sessionStorage across the navigation
            // so the finished ceremony still offers the share link.
            tonk_ui::register_dialog::stash_share(&request.space);
            if let Some(location) = web_sys::window().map(|window| window.location()) {
                let _ = location.assign("/settings");
            }
            return;
        }
        match return_focus {
            Some(return_focus) => tonk_ui::register_dialog::open_with_return_focus(move || {
                return_focus.restore();
            }),
            None => tonk_ui::register_dialog::open(),
        }
        tonk_ui::register_dialog::describe(reason);
        tonk_ui::register_dialog::adopt_stashed_share();
    });
    tonk_ui::account::register();
    tonk_ui::activate::register();

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    if let Err(error) = tonk_host::ready::require().await {
        web_sys::console::error_1(&error);
        show_readiness_failure();
        return;
    }
    mount_root();
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn show_readiness_failure() {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Ok(hook) = js_sys::Reflect::get(&window, &JsValue::from_str("tonkBootTerminal"))
        && let Ok(hook) = hook.dyn_into::<js_sys::Function>()
        && hook
            .call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str(READINESS_FAILURE_MESSAGE),
            )
            .is_ok()
    {
        return;
    }

    // Test harnesses and embeds may omit the boot-watchdog hook. Preserve the
    // same visible safe-state and next-action copy there.
    let Some(status) = window
        .document()
        .and_then(|document| document.query_selector("[data-boot-status]").ok().flatten())
    else {
        return;
    };
    let _ = status.set_attribute("data-failed", "");
    status.set_text_content(Some(READINESS_FAILURE_MESSAGE));
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
    // Redirect the OLD panel addresses onto their /account home; render the
    // panel when we are already there.
    let here = format!("{path}{search}");
    if let Some(canonical) = canonical_account.as_ref()
        && canonical != &here
    {
        let _ = window.location().replace(canonical);
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
    fn it_canonicalizes_account_routes_without_losing_query_parameters() {
        // The account panel's home is /account; legacy /settings deep
        // links redirect there. Bare /settings is the routed page now.
        assert_eq!(canonical_account_url("/settings", ""), None);
        assert_eq!(
            canonical_account_url("/settings", "?next=%2Fspace%2Fone"),
            Some("/account?next=%2Fspace%2Fone".into())
        );
        assert_eq!(
            canonical_account_url("/settings/link", "?callback=http%3A%2F%2Flocalhost"),
            Some("/account/link?callback=http%3A%2F%2Flocalhost".into())
        );
        assert_eq!(
            canonical_account_url("/account", "?revoke=did%3Akey%3Aone"),
            Some("/account?revoke=did%3Akey%3Aone".into())
        );
        assert_eq!(
            canonical_account_url("/account/link", "?audience=did%3Akey%3Acli"),
            Some("/account/link?audience=did%3Akey%3Acli".into())
        );
        assert_eq!(canonical_account_url("/space/one", ""), None);
    }
}
