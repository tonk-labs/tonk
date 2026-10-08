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
    // A delegation from the account is signed by the worker that holds the
    // account, behind a passkey this page asks for.
    tonk_portal::on_delegate(|request, reply| {
        tonk_ui::custody_relay::delegate(
            tonk_worker_api::RootDelegation {
                subject: request.subject,
                command: request.command,
                audience: request.audience,
            },
            move |answer| reply.finish(answer),
        );
    });
    // An account task is the profile frame's to answer. One that reaches
    // this page has no panel here to open.
    tonk_portal::on_task(|_request, reply| {
        if let Some(reply) = reply {
            reply.finish("invalid");
        }
    });

    // Dev-only hot reload client. `debug_assertions` is on under `trunk serve`
    // (debug profile) and off for release, so this never loads in production.
    #[cfg(debug_assertions)]
    inject_hot_swap();

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
    show_stages(&shell);
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
    let current = shell.first_element_child();

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

/// Say, where the boot shell reports progress, how far along the profile's
/// site is: its frame tells `<tonk-site>` each stage it reaches.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn show_stages(shell: &web_sys::Element) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let on_stage =
        Closure::<dyn FnMut(web_sys::CustomEvent)>::new(|event: web_sys::CustomEvent| {
            let Some(stage) = event.detail().as_string() else {
                return;
            };
            let status = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.query_selector("[data-boot-status]").ok().flatten());
            if let Some(status) = status
                && !status.has_attribute("data-failed")
            {
                // Nothing to say once the site is showing.
                let said = if stage == "ready" {
                    String::new()
                } else {
                    format!("{stage}…")
                };
                status.set_text_content(Some(&said));
            }
        });
    let _ = shell.add_event_listener_with_callback(
        tonk_portal::STAGE_EVENT,
        on_stage.as_ref().unchecked_ref(),
    );
    on_stage.forget();
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
