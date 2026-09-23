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

use std::str::FromStr;

use js_sys::{Array, Object, Reflect};
use tonk_host::bridge::{context_field, context_origin};
use tonk_host::location::Location;
use tonk_host::space_origin::encode_label;
use wasm_bindgen::JsValue;
use web_sys::{Element, HtmlIFrameElement, MessageChannel, MessagePort, Url, window};

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
