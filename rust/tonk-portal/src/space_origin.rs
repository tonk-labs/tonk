//! Sites on their own origin (proof of concept).
//!
//! Where the deployment configures a site host, a `<tonk-site>` renders in an
//! iframe at a real origin of its own instead of an opaque `srcdoc` frame: a
//! space at `{label}.{site host}`, the profile at `profile.{site host}`. The frame keeps `allow-same-origin`, which is safe only
//! because that origin is never its parent's: the guest cannot reach into its
//! parent to lift its own sandbox. What it gains is storage and a service
//! worker of its own.
//!
//! The profile chrome must be on a real origin too, even though it needs
//! neither: it is what nests the space frame, and a frame nested in an opaque
//! one inherits its sandbox and is opaque as well.
//!
//! The frame first loads a static shell ([`SHELL_PATH`]), which registers the
//! site's worker and then asks for its document. The host answers with the
//! same bootstrap markup a sealed frame receives as `srcdoc`, so the bridge
//! handshake and the runtime injection run unchanged.
//!
//! A space's worker serves `/blob/{hash}` itself, pulling the bytes from the
//! host worker over a `MessagePort`. Only the top document can reach the host
//! worker, so it mints every port, bound to the space and branch of the frame
//! that asked. A nested frame's request travels up through each portal, and
//! the top document grants it only if the asking portal's `allow` reaches the
//! requested space.
//!
//! A site origin that has never been loaded has no worker, so offline its
//! frame cannot load at all. The shell announces itself the moment it runs;
//! a frame that finishes loading without that is reported in place of the
//! site ([`watch_shell`]), with the reason and a way to try again.

use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use js_sys::{Array, Function, Object, Reflect};
use tonk_host::bridge::{context_field, context_origin};
use tonk_host::location::Location;
use tonk_host::space_origin::encode_label;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    AddEventListenerOptions, Element, HtmlElement, HtmlIFrameElement, MessageChannel, MessagePort,
    Url, window,
};

use crate::bridge::PortalState;
use crate::shared::reload_portal;

/// The static page every site origin loads first.
pub(crate) const SHELL_PATH: &str = "/space-origin.html";

/// The label of the profile's origin, `profile.{host}`. A space label is the
/// multibase encoding of its key, so it always starts with `b` and is far
/// longer than this: the two never collide.
const PROFILE_LABEL: &str = "profile";

/// Sandbox for a real-origin site frame. `allow-same-origin` keeps the frame's
/// own origin (storage, service worker). Top navigation and popups stay
/// withheld: they are egress paths that CSP `connect-src` does not cover.
/// `allow-forms` and `allow-downloads` match the sealed frame.
pub(crate) const SANDBOX: &str = "allow-scripts allow-same-origin allow-forms allow-downloads";

/// The authority `host` renders its site under, or `None` to keep it in a
/// sealed frame. Only a `<tonk-site>` renders on an origin of its own: the top
/// document's names the authority in its `origin` attribute (from the
/// deployment's configuration), and a site inside a real-origin guest takes
/// the one its parent handed it (`siteHost` in its context). Other portals
/// (`<tonk-portal>`, the FAB's) stay sealed.
pub(crate) fn site_host(host: &Element) -> Option<String> {
    if host.tag_name() != "TONK-SITE" {
        return None;
    }
    if let Some(site_host) = host
        .get_attribute("origin")
        .filter(|value| !value.is_empty())
    {
        return Some(site_host);
    }
    let own = window()?.location().origin().ok()?;
    (own != "null").then(|| context_field("siteHost")).flatten()
}

/// The real origin a site at `with` renders at: its label under `site_host`,
/// with the app's scheme. A space renders at `{label}.{site_host}` and the
/// profile at `profile.{site_host}`.
///
/// `None` for a location with no label, and for one that would share this
/// document's origin: a frame on its parent's origin, with
/// `allow-same-origin`, could lift its own sandbox.
pub(crate) fn site_origin(with: &Location, site_host: &str) -> Option<String> {
    let label = match with.space() {
        Some(space) => encode_label(space)?,
        None if with.profile() => PROFILE_LABEL.to_owned(),
        None => return None,
    };
    let app = Url::new(&context_origin()?).ok()?;
    let origin = format!("{}//{label}.{site_host}", app.protocol());
    let own = window()?.location().origin().ok()?;
    (origin != own).then_some(origin)
}

/// How long after its frame finished loading a shell may still announce
/// itself. The announcement is posted before the load ends but crosses
/// processes, so it can arrive a little after.
const SHELL_GRACE_MS: i32 = 1_000;

/// How long a frame may take to load its shell at all. The shell is one small
/// static page; this only bounds a browser that never reports the load.
const SHELL_TIMEOUT_MS: i32 = 15_000;

/// The class of the notice shown in place of a site that could not load.
const UNREACHABLE_CLASS: &str = "tonk-site-unreachable";

/// Watch the frame `host` renders its site in, loading for the `load`-th
/// time: if it finishes loading (or takes too long) without its shell having
/// announced itself, say in its place why the site cannot load.
pub(crate) fn watch_shell(
    host: &Element,
    iframe: &HtmlIFrameElement,
    state: &Rc<RefCell<PortalState>>,
    load: u32,
) {
    let check = {
        let host = host.clone();
        let iframe = iframe.clone();
        let state = state.clone();
        move || {
            let shell = state.borrow().shell.get();
            let shown = host.get_attribute("data-state").as_deref() == Some("unreachable");
            if shell.load == load && !shell.seen && !shown {
                show_unreachable(&host, &iframe, &state);
            }
        }
    };
    after(SHELL_TIMEOUT_MS, check.clone());
    let loaded = Closure::once_into_js(move || after(SHELL_GRACE_MS, check));
    let options = AddEventListenerOptions::new();
    options.set_once(true);
    let _ = iframe.add_event_listener_with_callback_and_add_event_listener_options(
        "load",
        loaded.unchecked_ref(),
        &options,
    );
}

/// Run `task` once, `ms` from now.
fn after(ms: i32, task: impl FnOnce() + 'static) {
    let Some(window) = window() else {
        return;
    };
    let task = Closure::once_into_js(task);
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(task.unchecked_ref(), ms);
}

/// Put a notice in place of the frame: why the site cannot load, and a way to
/// try again. Offline, coming back online tries again without being asked.
fn show_unreachable(host: &Element, iframe: &HtmlIFrameElement, state: &Rc<RefCell<PortalState>>) {
    clear_unreachable(iframe);
    let Some(window) = window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let origin = state.borrow().origin().unwrap_or_default().to_owned();
    let offline = !window.navigator().on_line();
    tonk_common::log!("site origin: {origin} could not load (offline: {offline})");
    let (title, detail) = if offline {
        (
            "Not available offline yet",
            "This site has not been opened on this device before, so it cannot load without a connection. It will load when you are back online.".to_owned(),
        )
    } else {
        (
            "This site could not load",
            format!("{origin} did not answer."),
        )
    };

    let Ok(notice) = document.create_element("div") else {
        return;
    };
    notice.set_class_name(UNREACHABLE_CLASS);
    let _ = notice.set_attribute("role", "alert");
    let _ = notice.set_attribute(
        "style",
        "display:grid;place-content:center;justify-items:center;gap:0.75rem;min-height:60vh;padding:2rem;box-sizing:border-box;text-align:center;font:inherit;color:inherit",
    );
    for (tag, text, style) in [
        ("strong", title, ""),
        ("span", detail.as_str(), "max-width:32rem;opacity:0.7"),
    ] {
        if let Ok(line) = document.create_element(tag) {
            line.set_text_content(Some(text));
            let _ = line.set_attribute("style", style);
            let _ = notice.append_child(&line);
        }
    }

    let retry = {
        let host = host.clone();
        let iframe = iframe.clone();
        let state = state.clone();
        move || {
            // Only while the notice it was made for still stands.
            if host.get_attribute("data-state").as_deref() == Some("unreachable") {
                clear_unreachable(&iframe);
                reload_portal(&host, &state);
            }
        }
    };
    if let Ok(button) = document.create_element("button") {
        button.set_text_content(Some("try again"));
        let _ = button.set_attribute("type", "button");
        let pressed: Function = Closure::once_into_js(retry.clone()).unchecked_into();
        let _ = button.add_event_listener_with_callback("click", &pressed);
        let _ = notice.append_child(&button);
    }
    if offline {
        let online: Function = Closure::once_into_js(retry).unchecked_into();
        let options = AddEventListenerOptions::new();
        options.set_once(true);
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            "online", &online, &options,
        );
    }

    iframe.set_hidden(true);
    let _ = host.set_attribute("data-state", "unreachable");
    let _ = host.append_child(&notice);
}

/// Take down the notice beside `iframe`, if one stands, and show the frame.
pub(crate) fn clear_unreachable(iframe: &HtmlIFrameElement) {
    let Some(host) = iframe.parent_element() else {
        return;
    };
    if host.get_attribute("data-state").as_deref() != Some("unreachable") {
        return;
    }
    let notice = format!(":scope > .{UNREACHABLE_CLASS}");
    while let Ok(Some(stale)) = host.query_selector(&notice) {
        stale.remove();
    }
    let _ = host.remove_attribute("data-state");
    let frame: &HtmlElement = iframe;
    frame.set_hidden(false);
}

/// Hand the frame its document once its shell reports that its worker is in
/// control. Targeted at the frame's origin, so the markup never lands in a
/// document that has navigated elsewhere.
pub(crate) fn deliver_document(iframe: &HtmlIFrameElement, origin: &str, html: &str) {
    let Some(target) = iframe.content_window() else {
        return;
    };
    let message = Object::new();
    let _ = Reflect::set(&message, &"__tonkOrigin".into(), &"document".into());
    let _ = Reflect::set(&message, &"html".into(), &JsValue::from_str(html));
    let _ = target.post_message(&message, origin);
}

/// Get the frame at `origin` a port to the host worker for `with`. The top
/// document mints it; any other document asks its parent, which answers with
/// a `relay-port` that [`relay_port`] passes down.
pub(crate) fn broker_port(iframe: &HtmlIFrameElement, origin: &str, with: &Location) {
    let Some(space) = with.space() else {
        return;
    };
    let Some(window) = window() else {
        return;
    };
    let top = window
        .parent()
        .ok()
        .flatten()
        .is_none_or(|parent| JsValue::from(parent) == JsValue::from(window.clone()));
    if top {
        if let Some(port) = mint_port(with) {
            post_port(iframe, origin, "port", with, port);
        }
        return;
    }
    let (Some(parent), Some(host)) = (window.parent().ok().flatten(), context_origin()) else {
        return;
    };
    let request = Object::new();
    let _ = Reflect::set(&request, &"__tonkOrigin".into(), &"need-port".into());
    let _ = Reflect::set(&request, &"repo".into(), &JsValue::from_str(space));
    let _ = Reflect::set(
        &request,
        &"branch".into(),
        &JsValue::from_str(with.effective_branch()),
    );
    let _ = parent.post_message(&request, &host);
}

/// Grant a nested frame's request, relayed by the portal at `iframe`: mint a
/// port for the location the request names, and send it down as a
/// `relay-port`. The caller has checked that the portal's `allow` reaches it.
pub(crate) fn grant_relayed_port(iframe: &HtmlIFrameElement, origin: &str, with: &Location) {
    if let Some(port) = mint_port(with) {
        post_port(iframe, origin, "relay-port", with, port);
    }
}

/// The location a relayed `need-port` or `relay-port` names.
pub(crate) fn requested_location(data: &JsValue) -> Option<Location> {
    let repo = Reflect::get(data, &"repo".into()).ok()?.as_string()?;
    let branch = Reflect::get(data, &"branch".into()).ok()?.as_string()?;
    Location::from_str(&format!("{branch}@{repo}")).ok()
}

/// Pass a port minted above down to the frame this portal renders `with` in.
pub(crate) fn relay_port(
    iframe: &HtmlIFrameElement,
    origin: &str,
    with: &Location,
    port: MessagePort,
) {
    post_port(iframe, origin, "port", with, port);
}

/// Create a channel to the host worker, bound to `with`'s space and branch,
/// and return the end the frame keeps. Only the top document has the host
/// worker as its controller.
fn mint_port(with: &Location) -> Option<MessagePort> {
    let space = with.space()?;
    let Some(controller) = window().and_then(|w| w.navigator().service_worker().controller())
    else {
        tonk_common::log!("site origin: no host worker to broker a port with");
        return None;
    };
    let channel = MessageChannel::new().ok()?;
    let bind = Object::new();
    let _ = Reflect::set(&bind, &"type".into(), &"space-port".into());
    let _ = Reflect::set(&bind, &"repo".into(), &JsValue::from_str(space));
    let _ = Reflect::set(
        &bind,
        &"branch".into(),
        &JsValue::from_str(with.effective_branch()),
    );
    if let Err(error) =
        controller.post_message_with_transferable(&bind, &Array::of1(&channel.port1()))
    {
        tonk_common::log!("site origin: host worker refused the port: {error:?}");
        return None;
    }
    Some(channel.port2())
}

fn post_port(
    iframe: &HtmlIFrameElement,
    origin: &str,
    kind: &str,
    with: &Location,
    port: MessagePort,
) {
    let Some(target) = iframe.content_window() else {
        return;
    };
    let message = Object::new();
    let _ = Reflect::set(&message, &"__tonkOrigin".into(), &JsValue::from_str(kind));
    if let Some(space) = with.space() {
        let _ = Reflect::set(&message, &"repo".into(), &JsValue::from_str(space));
    }
    let _ = Reflect::set(
        &message,
        &"branch".into(),
        &JsValue::from_str(with.effective_branch()),
    );
    let _ = target.post_message_with_transfer(&message, origin, &Array::of1(&port));
}
