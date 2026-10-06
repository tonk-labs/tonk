//! UI binary entrypoint.
//!
//! This binary initializes and mounts the Tonk UI component to the browser DOM.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="ui"` link tag).

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_worker_api::DeploymentConfig;
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

    // This page mounts one `<tonk-site>` for the profile, on the profile's
    // own origin, tells it where the address bar is, and runs the passkey
    // ceremonies the profile's worker asks for. Everything else (the hub,
    // a space's chrome, the bar, a space's content) renders in that site
    // and the sites it nests, and this page asks no worker for anything.
    tonk_portal::register_site();

    // What a frame asks of the page around it: navigate, set the title,
    // pass an analytics event on.
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
    // The panel that adds an account is the profile frame's own; what a
    // frame still asks of this page is where to seat the passkey rows a
    // settings command raised, beside the column that asked.
    tonk_portal::on_register(|reason, return_focus| {
        let request = tonk_ui::custody_relay::parse_seat_request(reason);
        if request.reason == "custody-anchor" {
            tonk_ui::custody_relay::return_to_approval(return_focus);
            if let Some(anchor) = request.anchor {
                tonk_ui::custody_relay::reanchor(anchor);
            }
        }
    });
    // An account task is the profile frame's to answer. One that reaches
    // this page has no panel here to open.
    tonk_portal::on_task(|_request, reply| {
        if let Some(reply) = reply {
            reply.finish("invalid");
        }
    });
    tonk_ui::activate::register();

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    mount_root();
    if web_sys::window().is_some_and(|window| {
        matches!(
            window.location().pathname().as_deref(),
            Ok("/activate" | "/activate/")
        )
    }) {
        tonk_ui::analytics::finish_startup(
            tonk_analytics::product::Stage::Ready,
            tonk_analytics::product::ProductResult::Success,
            None,
        );
    }
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
    let _ = status.set_attribute("role", "alert");
    status.set_text_content(Some(READINESS_FAILURE_MESSAGE));
}

/// Mount the top-document shell.
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
    let activate_route = path == "/activate" || path.starts_with("/activate/");
    let current = shell.first_element_child();

    // The activation email lands here on any device, signed in or not,
    // so the page bypasses sealed guests. Everything else, the account's
    // settings included, renders inside the routed site.
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

    // The profile renders on an origin of its own, which the deployment's
    // configuration names. A navigation while that is being read must not
    // mount a second site.
    if shell.has_attribute("data-mounting") {
        return;
    }
    let _ = shell.set_attribute("data-mounting", "");
    let shell = shell.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let site_pattern = site_pattern().await;
        let _ = shell.remove_attribute("data-mounting");
        let Some(site_pattern) = site_pattern else {
            show_readiness_failure();
            return;
        };
        shell.set_inner_html("");
        let Some(document) = shell.owner_document() else {
            return;
        };
        let Ok(site) = document.create_element("tonk-site") else {
            return;
        };
        let _ = site.set_attribute("with", &tonk_host::bridge::profile_with());
        let _ = site.set_attribute("allow", "*");
        let _ = site.set_attribute("origin", &site_pattern);
        // The path may have moved while the configuration was being read.
        let path = web_sys::window()
            .and_then(|window| window.location().pathname().ok())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".to_owned());
        let _ = site.set_attribute("path", &path);
        let _ = shell.append_child(&site);
    });
}

/// The hostname this deployment renders each site at, with `*` where the
/// site's label goes (`*.tonk.spot`), from its `/.well-known/tonk`. `None`
/// when it names none, or the configuration cannot be read.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn site_pattern() -> Option<String> {
    let origin = web_sys::window()?.location().origin().ok()?;
    let config: DeploymentConfig = reqwest::get(format!("{origin}/.well-known/tonk"))
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    config.sites.map(|sites| sites.pattern())
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
