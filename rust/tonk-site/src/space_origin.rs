//! Sites on their own origin (proof of concept).
//!
//! Where the deployment configures a site host, a `<tonk-site>` renders in an
//! iframe at a real origin of its own instead of an opaque `srcdoc` frame: a
//! space at `{label}.{site host}`, the profile at `profile.{site host}`. A
//! deployment that shares its zone puts a suffix after the label, so a pull
//! request's preview has `{label}-pr33.{site host}`. The frame keeps
//! `allow-same-origin`, which is safe only because that origin is never its
//! parent's: the guest cannot reach into its
//! parent to lift its own sandbox. What it gains is storage and a service
//! worker of its own.
//!
//! The profile chrome must be on a real origin too, even though it needs
//! neither: it is what nests the space frame, and a frame nested in an opaque
//! one inherits its sandbox and is opaque as well.
//!
//! The frame first loads a static shell ([`shell_url`]), which registers the
//! site's worker and then asks for its document. The host answers with the
//! same bootstrap markup a sealed frame receives as `srcdoc`, so the bridge
//! handshake and the runtime injection run unchanged.
//!
//! A space's worker serves `/asset:{hash}` itself, from the space's own
//! database. What only the host has (the space's delegation, the snapshot a
//! fresh replica seeds from) it asks the host worker for over a
//! `MessagePort`. Only the top document can reach the host
//! worker, so it mints every port, bound to the space and branch of the frame
//! that asked. A nested frame's request travels up through each portal, and
//! the top document grants it only if the asking portal's `allow` reaches the
//! requested space.
//!
//! A site origin that has never been loaded has no worker, so offline its
//! frame cannot load at all. The shell announces itself the moment it runs;
//! a frame that finishes loading without that is reported in place of the
//! site ([`watch_shell`]), with the reason and a way to try again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;

use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use tonk_host::bridge::{context_field, context_origin};
use tonk_host::location::Location;
use tonk_host::space_origin::{encode_label, site_hostname};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AddEventListenerOptions, Element, HtmlElement, HtmlIFrameElement, MessageChannel, MessageEvent,
    MessagePort, Url, window,
};

use crate::bridge::{PortalState, fetch_text};
use crate::site::reload_site;

/// The static page a site's origin loads first, the profile's as a space's.
const SITE_SHELL: &str = "/site.html";

/// What the deployment ships: the kernel it installs into a space it makes,
/// `kernel`, and the persona a new profile runs, `persona`.
const SHIPPED: &str = "/_host/kernel.json";

/// Tell the shell of the new site at `origin`, framed by `iframe`, which
/// kernel it runs: the deployment's persona for the person's profile, and
/// for a space the kernel the profile installs into one it makes. This page
/// framed the site, so it knows which the site is; the shell never tells by
/// its hostname. It says too whether it hands the site a port up to a host,
/// `hosted`, as it does a space: the site's worker then takes its kernel from
/// that host, and waits for the port to do so.
pub(crate) fn name_kernel(iframe: &HtmlIFrameElement, origin: &str, profile: bool, hosted: bool) {
    let (iframe, origin) = (iframe.clone(), origin.to_owned());
    spawn_local(async move {
        let field = if profile { "persona" } else { "kernel" };
        let shipped = match fetch_text(SHIPPED).await {
            Ok(shipped) => shipped,
            Err(error) => {
                tonk_common::log!("tonk-site: {SHIPPED}: {error}");
                return;
            }
        };
        let Some(kernel) = JSON::parse(&shipped)
            .ok()
            .and_then(|shipped| Reflect::get(&shipped, &field.into()).ok())
            .and_then(|kernel| kernel.as_string())
        else {
            tonk_common::log!("tonk-site: {SHIPPED} names no {field}");
            return;
        };
        let Some(target) = iframe.content_window() else {
            return;
        };
        let message = Object::new();
        let _ = Reflect::set(&message, &"__tonkOrigin".into(), &"kernel".into());
        let _ = Reflect::set(&message, &"kernel".into(), &JsValue::from_str(&kernel));
        let _ = Reflect::set(&message, &"host".into(), &JsValue::from_bool(hosted));
        let _ = target.post_message(&message, &origin);
    });
}

/// Let the top page tell the profile's worker what came of a passkey
/// ceremony it ran, through `iframe`, the profile's frame on `origin`: what
/// a custody ceremony derived, or that a ceremony was refused. The ceremony
/// runs here, where the passkeys are; the account it opens is the profile's
/// worker's. `tonkProfileWorker(message, transfer)` posts it. Nothing to
/// install in a nested document.
pub(crate) fn expose_profile_worker(iframe: &HtmlIFrameElement, origin: &str) {
    let Some(window) = window() else {
        return;
    };
    let nested = window
        .parent()
        .ok()
        .flatten()
        .is_some_and(|parent| JsValue::from(parent) != JsValue::from(window.clone()));
    if nested {
        return;
    }
    let iframe = iframe.clone();
    let origin = origin.to_owned();
    let relay =
        Closure::<dyn Fn(JsValue, JsValue)>::new(move |message: JsValue, transfer: JsValue| {
            let Some(frame) = iframe.content_window() else {
                return;
            };
            let envelope = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&envelope, &"__tonkOrigin".into(), &"to-worker".into());
            let _ = js_sys::Reflect::set(&envelope, &"message".into(), &message);
            let _ = frame.post_message_with_transfer(&envelope, &origin, &transfer);
        });
    let _ = js_sys::Reflect::set(&window, &"tonkProfileWorker".into(), relay.as_ref());
    relay.forget();
}

/// The address of `path` in the site at `origin`: what its frame loads. The
/// site's worker answers it with the site's shell, or with whatever content
/// the site keeps at that path.
pub(crate) fn site_url(origin: &str, path: &str) -> String {
    let path = path.trim_start_matches('/');
    format!("{origin}/{path}")
}

/// The page every address of `origin` is answered with where a route shows
/// a model: what a frame that only reaches the origin's worker loads.
pub(crate) fn shell_url(origin: &str) -> String {
    format!("{origin}{SITE_SHELL}")
}

/// The label of the profile's origin, `profile.{host}`. A space label is the
/// multibase encoding of its key, so it always starts with `b` and is far
/// longer than this: the two never collide.
const PROFILE_LABEL: &str = "profile";

/// Sandbox for a real-origin site frame. `allow-same-origin` keeps the frame's
/// own origin (storage, service worker). Top navigation and popups stay
/// withheld: they are egress paths that CSP `connect-src` does not cover.
/// `allow-forms` and `allow-downloads` match the sealed frame.
pub(crate) const SANDBOX: &str = "allow-scripts allow-same-origin allow-forms allow-downloads";

/// The hostname `host` renders its site at, with `*` where the site's label
/// goes (`*.tonk.spot`), or `None` to keep it in a sealed frame. Only a
/// `<tonk-site>` renders on an origin of its own: the top document's names
/// the pattern in its `origin` attribute (from the deployment's
/// configuration), and a site inside a real-origin guest takes the one its
/// parent handed it (`sitePattern` in its context). Any other host stays
/// sealed.
pub(crate) fn site_pattern(host: &Element) -> Option<String> {
    if host.tag_name() != "TONK-SITE" {
        return None;
    }
    if let Some(site_pattern) = host
        .get_attribute("origin")
        .filter(|value| !value.is_empty())
    {
        return Some(site_pattern);
    }
    let own = window()?.location().origin().ok()?;
    (own != "null")
        .then(|| context_field("sitePattern"))
        .flatten()
}

/// The real origin a site at `with` renders at: `site_pattern` with the
/// site's label in place of its `*`, and the app's scheme. Under `*.tonk.spot`
/// a space renders at `{label}.tonk.spot` and the profile at
/// `profile.tonk.spot`.
///
/// `None` for a location with no label, and for one that would share this
/// document's origin: a frame on its parent's origin, with
/// `allow-same-origin`, could lift its own sandbox.
pub(crate) fn site_origin(with: &Location, site_pattern: &str) -> Option<String> {
    let label = match with.space() {
        Some(space) => encode_label(space)?,
        None if with.profile() => PROFILE_LABEL.to_owned(),
        None => return None,
    };
    let app = Url::new(&context_origin()?).ok()?;
    let origin = format!(
        "{}//{}",
        app.protocol(),
        site_hostname(site_pattern, &label)
    );
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
                reload_site(&host, &state);
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

thread_local! {
    /// What this page's worker said its site can do, once it said.
    static HOSTS_SPACES: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Whether this document's site hosts the person's spaces, as its worker
/// says: a persona's does, and so its worker is the one a space's worker
/// asks for its delegation. Asked once in a page's life: the kernel a site
/// runs does not change under it. A page no worker answers hosts none.
async fn hosts_spaces() -> bool {
    if let Some(known) = HOSTS_SPACES.get() {
        return known;
    }
    let Some(controller) =
        window().and_then(|window| window.navigator().service_worker().controller())
    else {
        return false;
    };
    let Ok(channel) = MessageChannel::new() else {
        return false;
    };
    let reply = channel.port1();
    let answered = Promise::new(&mut |resolve, _| reply.set_onmessage(Some(&resolve)));
    let ask = Object::new();
    let _ = Reflect::set(&ask, &"type".into(), &"abilities".into());
    if controller
        .post_message_with_transferable(&ask, &Array::of1(&channel.port2()))
        .is_err()
    {
        return false;
    }
    let hosts = JsFuture::from(answered)
        .await
        .ok()
        .and_then(|event| Reflect::get(&event, &"data".into()).ok())
        .and_then(|data| Reflect::get(&data, &"hostsSpaces".into()).ok())
        .and_then(|hosts| hosts.as_bool())
        .unwrap_or(false);
    HOSTS_SPACES.set(Some(hosts));
    hosts
}

/// Whether this document is the tab's own, framed by nothing.
pub(crate) fn on_top() -> bool {
    window().is_some_and(|window| {
        window
            .parent()
            .ok()
            .flatten()
            .is_none_or(|parent| JsValue::from(parent) == JsValue::from(window))
    })
}

/// Get the frame at `origin` a port to the profile's worker for `with`. The
/// app's page, showing a space itself, has the profile's origin mint it in a
/// frame of its own; a document whose site hosts the person's spaces mints
/// it with its own worker; any other asks its parent, which answers with a
/// `relay-port` that [`relay_port`] passes down.
pub(crate) fn broker_port(iframe: &HtmlIFrameElement, origin: &str, with: &Location) {
    if with.space().is_none() {
        return;
    }
    if on_top() {
        let (frame, origin, with) = (iframe.clone(), origin.to_owned(), with.clone());
        let Some(pattern) = iframe.parent_element().and_then(|host| site_pattern(&host)) else {
            return;
        };
        connector::mint(&pattern, &with.clone(), move |port| {
            post_port(&frame, &origin, "port", &with, port);
        });
        return;
    }
    let (iframe, origin, with) = (iframe.clone(), origin.to_owned(), with.clone());
    spawn_local(async move {
        if hosts_spaces().await {
            if let Some(port) = mint_port(&with) {
                post_port(&iframe, &origin, "port", &with, port);
            }
            return;
        }
        ask_parent_for_port(&with);
    });
}

/// Ask the page around this one for a port for `with`, as a frame nested in
/// it would.
fn ask_parent_for_port(with: &Location) {
    let Some(space) = with.space() else {
        return;
    };
    let Some(window) = window() else {
        return;
    };
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

/// Whether `did`, the first segment of an app address, names one of the
/// person's accounts, asked of the profile's origin where sites render at
/// `pattern`. An account not signed in to is switched to first, so the
/// profile the app shows is that account's; the frame that asked is then
/// let go, since the profile renders as the app's site. `answered` hears
/// `false` for anything else, a space.
pub fn account(pattern: &str, did: &str, answered: impl FnOnce(bool) + 'static) {
    connector::account(pattern, did, answered);
}

/// Whether the person is signed in to an account, asked of the profile's
/// origin where sites render at `pattern`: what a space the app shows
/// itself needs before it asks the account for anything.
pub fn signed_in(pattern: &str, answered: impl FnOnce(bool) + 'static) {
    connector::signed_in(pattern, answered);
}

/// The profile's origin, framed unseen by the app's page while it shows a
/// space itself: only the profile's worker answers a space's worker for the
/// person's persona, and only a page of its origin opens a port to it. The
/// frame comes up the first time it is asked something, and answers each
/// ask by its id.
mod connector {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::str::FromStr;

    use js_sys::{Object, Reflect};
    use tonk_host::bridge::profile_with;
    use tonk_host::location::Location;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{HtmlIFrameElement, MessageEvent, MessagePort, window};

    use super::{expose_profile_worker, shell_url, site_origin};

    type Answered = Box<dyn FnOnce(&MessageEvent)>;

    struct Connector {
        iframe: HtmlIFrameElement,
        origin: String,
        ready: bool,
        /// Asked before the frame was ready, to ask once it is.
        waiting: Vec<(Object, Answered)>,
        asked: HashMap<u32, Answered>,
        next: u32,
    }

    thread_local! {
        static CONNECTOR: RefCell<Option<Connector>> = const { RefCell::new(None) };
    }

    /// Have the profile's origin, where sites render at `pattern`, mint a
    /// port to its worker for `with`, and hand it to `minted`.
    pub(super) fn mint(pattern: &str, with: &Location, minted: impl FnOnce(MessagePort) + 'static) {
        let message = said("mint-port");
        if let Some(space) = with.space() {
            let _ = Reflect::set(&message, &"repo".into(), &JsValue::from_str(space));
        }
        let _ = Reflect::set(
            &message,
            &"branch".into(),
            &JsValue::from_str(with.effective_branch()),
        );
        send(pattern, message, move |event: &MessageEvent| {
            if let Ok(port) = event.ports().get(0).dyn_into::<MessagePort>() {
                minted(port);
            }
        });
    }

    /// See [`super::account`].
    pub(super) fn account(pattern: &str, did: &str, answered: impl FnOnce(bool) + 'static) {
        let message = said("account");
        let _ = Reflect::set(&message, &"did".into(), &JsValue::from_str(did));
        send(pattern, message, move |event: &MessageEvent| {
            let account = Reflect::get(&event.data(), &"account".into())
                .ok()
                .and_then(|account| account.as_bool())
                .unwrap_or(false);
            if account {
                close();
            }
            answered(account);
        });
    }

    /// See [`super::signed_in`].
    pub(super) fn signed_in(pattern: &str, answered: impl FnOnce(bool) + 'static) {
        send(pattern, said("signed-in"), move |event: &MessageEvent| {
            answered(
                Reflect::get(&event.data(), &"signedIn".into())
                    .ok()
                    .and_then(|signed_in| signed_in.as_bool())
                    .unwrap_or(false),
            );
        });
    }

    fn said(what: &str) -> Object {
        let message = Object::new();
        let _ = Reflect::set(&message, &"__tonkOrigin".into(), &JsValue::from_str(what));
        message
    }

    fn send(pattern: &str, message: Object, answered: impl FnOnce(&MessageEvent) + 'static) {
        let answered: Answered = Box::new(answered);
        CONNECTOR.with_borrow_mut(|connector| {
            if connector.is_none() {
                *connector = open(pattern);
            }
            let Some(connector) = connector.as_mut() else {
                tonk_common::log!("site origin: the profile's origin could not be framed");
                return;
            };
            if connector.ready {
                ask(connector, message, answered);
            } else {
                connector.waiting.push((message, answered));
            }
        });
    }

    fn open(pattern: &str) -> Option<Connector> {
        let profile = Location::from_str(&profile_with()).ok()?;
        let origin = site_origin(&profile, pattern)?;
        let document = window()?.document()?;
        let iframe = document
            .create_element("iframe")
            .ok()?
            .dyn_into::<HtmlIFrameElement>()
            .ok()?;
        iframe.set_hidden(true);
        let _ = iframe.set_attribute("sandbox", "allow-scripts allow-same-origin");
        let _ = iframe.set_attribute("title", "profile");
        iframe.set_src(&format!("{}#connector", shell_url(&origin)));
        hear(&iframe, &origin);
        // What a passkey ceremony this page runs for the space derived is
        // the profile's worker's to hear, through this frame.
        expose_profile_worker(&iframe, &origin);
        document.body()?.append_child(&iframe).ok()?;
        Some(Connector {
            iframe,
            origin,
            ready: false,
            waiting: Vec::new(),
            asked: HashMap::new(),
            next: 0,
        })
    }

    /// Let the frame go: what it was asked for is no longer the app's to
    /// ask.
    fn close() {
        if let Some(connector) = CONNECTOR.with_borrow_mut(Option::take) {
            connector.iframe.remove();
        }
    }

    fn ask(connector: &mut Connector, message: Object, answered: Answered) {
        let Some(target) = connector.iframe.content_window() else {
            return;
        };
        let id = connector.next;
        connector.next += 1;
        connector.asked.insert(id, answered);
        let _ = Reflect::set(&message, &"id".into(), &id.into());
        let _ = target.post_message(&message, &connector.origin);
    }

    /// What the frame says: that it is ready to be asked, and each answer.
    /// Heard only from the frame, on the profile's origin.
    fn hear(iframe: &HtmlIFrameElement, origin: &str) {
        let Some(window) = window() else {
            return;
        };
        let (iframe, origin) = (iframe.clone(), origin.to_owned());
        let heard = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let from_frame = iframe
                .content_window()
                .zip(event.source())
                .is_some_and(|(frame, source)| JsValue::from(frame) == JsValue::from(source));
            if !from_frame || event.origin() != origin {
                return;
            }
            let data = event.data();
            let said = Reflect::get(&data, &"__tonkOrigin".into())
                .ok()
                .and_then(|said| said.as_string());
            match said.as_deref() {
                // The frame is the profile's: a new one runs the persona,
                // and has no host.
                Some("need-kernel") => super::name_kernel(&iframe, &origin, true, false),
                Some("connector-ready") => CONNECTOR.with_borrow_mut(|connector| {
                    let Some(connector) = connector.as_mut() else {
                        return;
                    };
                    connector.ready = true;
                    for (message, answered) in std::mem::take(&mut connector.waiting) {
                        ask(connector, message, answered);
                    }
                }),
                Some("answered") => {
                    let answered = Reflect::get(&data, &"id".into())
                        .ok()
                        .and_then(|id| id.as_f64())
                        .and_then(|id| {
                            CONNECTOR.with_borrow_mut(|connector| {
                                connector
                                    .as_mut()
                                    .and_then(|connector| connector.asked.remove(&(id as u32)))
                            })
                        });
                    if let Some(answered) = answered {
                        answered(&event);
                    }
                }
                _ => {}
            }
        });
        let _ = window.add_event_listener_with_callback("message", heard.as_ref().unchecked_ref());
        heard.forget();
    }
}

/// A frame this page keeps, unseen, on the origin of a space that is not on
/// screen: only to reach the space's worker.
struct Reached {
    iframe: HtmlIFrameElement,
    origin: String,
    with: Location,
    /// Whether its shell has asked for a port, and so can take one.
    listening: bool,
}

thread_local! {
    static REACHED: RefCell<HashMap<String, Reached>> = RefCell::new(HashMap::new());
    static REACHING: Cell<bool> = const { Cell::new(false) };
}

/// Reach the workers of spaces that are not on screen, when this page's
/// worker asks.
///
/// A site that hosts the person's spaces, the profile, holds none of a
/// space's content and passes what it is asked about one to the space's own
/// worker, over a port only a page can open. Around a space on screen its
/// `<tonk-site>` opens it. For any other the worker asks this page
/// (`need-space-port`), which frames the space's origin unseen and opens one
/// through that frame, and drops the frame when the worker lets the space
/// go. Only such a worker asks, so on any other site this waits for what
/// never comes. Idempotent.
pub(crate) fn install_reach() {
    if REACHING.replace(true) {
        return;
    }
    let Some(window) = window() else {
        return;
    };
    let asked = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        let data = event.data();
        let kind = Reflect::get(&data, &"type".into())
            .ok()
            .and_then(|kind| kind.as_string());
        let Some(with) = requested_location(&data) else {
            return;
        };
        match kind.as_deref() {
            Some("need-space-port") => reach(&with),
            Some("release-space-port") => release(&with),
            _ => {}
        }
    });
    let _ = window
        .navigator()
        .service_worker()
        .add_event_listener_with_callback("message", asked.as_ref().unchecked_ref());
    asked.forget();

    // The frame's shell asks for a port once its worker is in control, and
    // again whenever that worker has restarted.
    // A new space's shell asks first which kernel it runs.
    let needs = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
        let data = event.data();
        let kind = Reflect::get(&data, &"__tonkOrigin".into())
            .ok()
            .and_then(|kind| kind.as_string());
        if !matches!(kind.as_deref(), Some("need-port" | "need-kernel")) {
            return;
        }
        let source = Reflect::get(&event, &"source".into()).unwrap_or(JsValue::NULL);
        REACHED.with_borrow_mut(|reached| {
            let frame = reached.values_mut().find(|frame| {
                frame.origin == event.origin()
                    && frame
                        .iframe
                        .content_window()
                        .is_some_and(|window| JsValue::from(window) == source)
            });
            let Some(frame) = frame else {
                return;
            };
            if kind.as_deref() == Some("need-kernel") {
                // A space's frame this page hands a port, once it asks.
                name_kernel(&frame.iframe, &frame.origin, false, true);
            } else {
                frame.listening = true;
                hand_port(frame);
            }
        });
    });
    let _ = window.add_event_listener_with_callback("message", needs.as_ref().unchecked_ref());
    needs.forget();
}

fn hand_port(frame: &Reached) {
    if let Some(port) = mint_port(&frame.with) {
        post_port(&frame.iframe, &frame.origin, "port", &frame.with, port);
    }
}

/// Open a port to the worker of the space at `with`: through the frame
/// already kept for it, or a new one.
fn reach(with: &Location) {
    let Some(space) = with.space() else {
        return;
    };
    let kept = REACHED.with_borrow(|reached| {
        reached.get(space).map(|frame| {
            if frame.listening {
                hand_port(frame);
            }
        })
    });
    // A space on screen has a frame already, whose shell opens the port as
    // it loads.
    if kept.is_some() || crate::bridge::renders(with) {
        return;
    }
    // Before this page has its context it does not know where sites render:
    // the worker asks again.
    let Some(origin) =
        context_field("sitePattern").and_then(|site_pattern| site_origin(with, &site_pattern))
    else {
        return;
    };
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Some(iframe) = document
        .create_element("iframe")
        .ok()
        .and_then(|element| element.dyn_into::<HtmlIFrameElement>().ok())
    else {
        return;
    };
    iframe.set_hidden(true);
    let _ = iframe.set_attribute("sandbox", "allow-scripts allow-same-origin");
    let _ = iframe.set_attribute("title", "space");
    iframe.set_src(&format!("{}#connector", shell_url(&origin)));
    let Some(body) = document.body() else {
        return;
    };
    let _ = body.append_child(&iframe);
    REACHED.with_borrow_mut(|reached| {
        reached.insert(
            space.to_owned(),
            Reached {
                iframe,
                origin,
                with: with.clone(),
                listening: false,
            },
        );
    });
}

/// Drop the frame kept for the space at `with`: its worker is no longer
/// being asked anything.
fn release(with: &Location) {
    let Some(space) = with.space() else {
        return;
    };
    if let Some(frame) = REACHED.with_borrow_mut(|reached| reached.remove(space)) {
        frame.iframe.remove();
    }
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

/// Create a channel to the profile's worker, bound to `with`'s space and
/// branch, and return the end the frame keeps. Only a document on the
/// profile's origin has that worker as its controller.
fn mint_port(with: &Location) -> Option<MessagePort> {
    let space = with.space()?;
    let Some(controller) = window().and_then(|w| w.navigator().service_worker().controller())
    else {
        tonk_common::log!("site origin: no profile worker to broker a port with");
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
        tonk_common::log!("site origin: the profile's worker refused the port: {error:?}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_addresses_a_path_in_a_site_at_the_sites_origin() {
        let origin = "https://label.tonk.test";
        assert_eq!(site_url(origin, "/"), "https://label.tonk.test/");
        assert_eq!(site_url(origin, ""), "https://label.tonk.test/");
        assert_eq!(site_url(origin, "/notes"), "https://label.tonk.test/notes");
        // A route template writes the rest of a path without its slash.
        assert_eq!(
            site_url(origin, "inspector/x"),
            "https://label.tonk.test/inspector/x"
        );
    }
}
