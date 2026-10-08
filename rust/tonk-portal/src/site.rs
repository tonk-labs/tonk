//! `<tonk-site with="…" allow="…" path="…">`: the routing element.
//!
//! `<tonk-site>` frames a site on an origin of its own and tells it which
//! path to show. It is the recursive unit of the UI: the top page mounts one
//! for the profile, the profile's chrome mounts a nested one for a space,
//! and each owns one isolation boundary.
//!
//! `with` (the site, `branch@repo`) and `allow` (the reach its guest may
//! ask for: `*`, `self`, or explicit locations) are **both required**: a
//! site missing or malforming either renders a visible error at connect, and
//! so does one the deployment names no origin for. There is no inheritance
//! and no defaulting: every site is fully self-describing, so privilege
//! never leaks downward.
//!
//! Flow on connect (and on navigation):
//! 1. Parse `with` + `allow`; take the path from the `path` attribute (a
//!    nested router; the top page's mount keeps it synced to the location).
//! 2. Bring up the frame on the site's origin via [`connect_portal`], with
//!    the site's entity and path in its context.
//! 3. The frame claims the transient `tonk:load` against its own worker,
//!    which stamps `tonk:site` (matching the path against its `route!`
//!    table). The frame's `<tonk-display>` subscribes to the site and
//!    renders the matched `{concept}`; a later path change is a new context
//!    and a new claim, and the subscription re-renders.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};
use crate::space_origin::{
    SANDBOX, expose_profile_worker, site_origin, site_pattern, site_url, watch_shell,
};

/// Shared cell holding the portal state once the iframe is up. An `Rc` so the
/// async site-registration task can hold it across the await and hand it to
/// `connect_portal`.
type StateCell = Rc<RefCell<Option<Rc<RefCell<PortalState>>>>>;

/// The `<tonk-site>` element. Holds the shared [`PortalState`] once its iframe is
/// up (`None` until the async site registration completes).
#[derive(Default)]
pub(crate) struct TonkSite {
    inner: StateCell,
}

impl CustomElement for TonkSite {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["path", "with", "allow"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        resolve_and_render(this, self.inner.clone());
        // Navigation is just a `path` attribute change, handled by
        // `attribute_changed_callback`. The element never reads `window.location`;
        // whoever mounts the top-level site (`ui.rs`) owns updating `path` on URL
        // change, so this element stays uniform for top-level and nested alike.
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        teardown(&self.inner);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // Re-resolve on a client-side navigation or a re-stamped context: the
        // chrome sets/updates/removes `path` in place when the route's `{rest}`
        // changes (e.g. `/space/{id}/inspector` → `/space/{id}` removes it), and
        // a template re-stamp can rewrite `with`/`allow`. The `<tonk-site>`
        // re-asserts `tonk:load` against the same site entity so the live
        // subscription re-renders.
        //
        // `with`/`allow` skip the first-set callback (the initial values are
        // handled by `connected_callback`) — NOTE the `custom-elements` JS shim
        // coerces a null `oldValue` to `""`, so a first set arrives as
        // `Some("")`, never `None`. `path` must NOT apply that skip: an absent
        // path is a routed state (`/`), so the empty → value transition IS a
        // navigation (the bare space route gaining a `{rest}` sub-path), not
        // mount noise. Pre-connect callbacks are already no-ops inside
        // `resolve_and_render` (`is_connected` gate), and a mount-time double
        // resolve is absorbed by the same-route iframe reuse.
        let first_set = old.as_deref().is_none_or(str::is_empty);
        let re_route = match name.as_str() {
            "path" => old != new,
            "with" | "allow" => !first_set && old != new,
            _ => false,
        };
        if re_route {
            resolve_and_render(this, self.inner.clone());
        }
    }
}

/// Tear down the portal iframe held by `cell`, if any.
///
/// TWO-PHASE: sever the comms (aborts + port closes via `sever`),
/// unload the guest realm so it tears down on its own schedule, and only
/// remove the element a tick later. Synchronously destroying a live nested
/// guest (running wasm, brokered ports, its own nested frames) from inside a
/// render pass is the pattern the browser process has crashed under — give
/// the unload a turn to settle first.
///
/// The unload goes through the frame's own `location.replace()`, NOT through
/// `iframe.src = "about:blank"`. Setting `src` *navigates* the frame, and a
/// frame navigation appends an entry to the JOINT session history — so every
/// teardown left a Back step behind, and the user had to press Back several
/// times to leave a page they had navigated to once. `location.replace()`
/// unloads the realm while replacing the current entry rather than adding one.
fn teardown(cell: &StateCell) {
    if let Some(state) = cell.borrow_mut().take() {
        let mut s = state.borrow_mut();
        s.disposed = true;
        s.sever();
        if let Some(iframe) = s.iframe.take() {
            crate::bridge::unregister_portal(&iframe);
            let _ = iframe.remove_attribute("srcdoc");
            // Replace (don't push) the frame's entry. If the content window is
            // unreachable (already detached), the frame is on its way out
            // anyway and the element removal below finishes the job.
            if let Some(frame_window) = iframe.content_window() {
                let _ = frame_window.location().replace("about:blank");
            }
            spawn_local(async move {
                let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                    if let Some(win) = window() {
                        let _ = win
                            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 100);
                    }
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                if let Some(parent) = iframe.parent_node() {
                    let _ = parent.remove_child(&iframe);
                }
            });
        }
    }
}

/// Read and parse a required routing attribute off the site. `Ok(None)`
/// means an unresolved template placeholder (skip this render — the real
/// frame re-sets the attribute and re-runs); `Err` is the visible-error
/// case (missing or malformed).
fn required_attribute<T: std::str::FromStr>(
    host: &HtmlElement,
    name: &str,
) -> Result<Option<T>, String>
where
    T::Err: std::fmt::Display,
{
    let Some(value) = host.get_attribute(name).filter(|v| !v.is_empty()) else {
        return Err(format!("missing required {name} attribute"));
    };
    if value.contains('{') {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|error| format!("malformed {name}={value:?}: {error}"))
}

/// Render a visible mount error into the site element. A site missing or
/// malforming `with`/`allow` must fail loudly at connect — never a silent
/// deny at query time.
fn render_site_error(host: &HtmlElement, message: &str) {
    tonk_common::log!("tonk-site: {message}");
    host.set_text_content(Some(&format!("tonk-site: {message}")));
    let _ = host.set_attribute("data-state", "malformed");
}

/// Resolve the path + the `with`/`allow` attributes, register the site, and
/// bring up the frame rendering the matched route. A missing or
/// malformed `with`/`allow` renders a visible error; a failed registration
/// leaves the element empty rather than throwing.
fn resolve_and_render(this: &HtmlElement, cell: StateCell) {
    let host = this.clone();

    // Attribute callbacks fire on `setAttribute` even before the element is
    // connected, i.e. mid-way through the mounter writing the attribute set.
    // Only a connected site renders; `connected_callback` runs the first
    // real pass once the element (with all its attributes) is in the tree.
    if !host.is_connected() {
        return;
    }

    // Both routing attributes are REQUIRED: every site is fully
    // self-describing (no inheritance, no defaulting), so privilege never
    // leaks downward. An unresolved `{…}` placeholder in either skips this
    // render; the stamped frame re-sets the attribute and re-runs.
    let with: Location = match required_attribute(&host, "with") {
        Ok(Some(with)) => with,
        Ok(None) => return,
        Err(message) => return render_site_error(&host, &message),
    };
    let allow: Allow = match required_attribute(&host, "allow") {
        Ok(Some(allow)) => allow,
        Ok(None) => return,
        Err(message) => return render_site_error(&host, &message),
    };
    // A site renders in a frame on an origin of its own, whose worker holds
    // its data and stamps its route. Without one there is nowhere to render.
    if !on_own_origin(&host, &with) {
        return render_site_error(
            &host,
            "this deployment names no origin for sites to render on",
        );
    }
    // Clear a previous pass's visible error (a re-stamp can heal a
    // malformed site) so the error text never lingers next to the iframe.
    if host.get_attribute("data-state").as_deref() == Some("malformed") {
        host.set_text_content(None);
        let _ = host.remove_attribute("data-state");
    }
    // `<tonk-site>` routes the `path` attribute it is given, which the frame
    // reads from its context. It NEVER reads `window.location` itself: the
    // top-level mount is given the document path explicitly (by `ui.rs`), so
    // the element is uniform, top-level and nested alike.
    //
    // A `{…}` left in the path is an unresolved template (a partial
    // substitution mid-render before the real frame lands). Skip; the real
    // frame re-sets `path` and re-runs this.
    if host
        .get_attribute("path")
        .is_some_and(|path| path.contains('{'))
    {
        return;
    }

    // The site entity this element renders against. Minted ONCE (reused across
    // re-resolves/navigations) so a navigation re-claims `tonk:load` against the
    // same entity: the cardinality-one `tonk:site` fields supersede in place and
    // the live subscription re-renders, no teardown. Per-element, so two
    // `<tonk-site>`s on one page never share a site entity (even on one branch).
    //
    // The frame claims the load itself, against its own worker, from the
    // entity and path this element hands it in its context (see
    // `bootstrap.js`), and claims it again whenever that worker has lost the
    // stamp.
    site_entity(&host);
    // Defer the iframe bring-up off this turn. `resolve_and_render` runs
    // synchronously inside `connected_callback` / `attribute_changed_callback`,
    // and `render_in_iframe` tears down + rebuilds the portal (touching the
    // bridge). Doing that synchronously inside a custom-element callback
    // re-enters the single-threaded lock the `custom_elements` runtime holds
    // across the callback → "cannot recursively acquire mutex". A `spawn_local`
    // lets the callback return first, so the render happens on a clean stack.
    //
    // A pure path change re-routes IN PLACE: with a live iframe already
    // connected under the same `with`/`allow`, only the `tonk:load` re-claim
    // is needed — the guest's `tonk:site` subscription delivers the new
    // route's frame and the render diff restamps the chrome's bindings
    // (the nested `<tonk-site with="main@{id}">`, the FAB's
    // `data-space={id}`) inside the running guest. Rebuilding here would
    // throw away the booted wasm on every navigation. Only a reach change
    // (different `with`/`allow`) or a missing/dead iframe rebuilds.
    let reuse = cell.borrow().as_ref().is_some_and(|state| {
        let s = state.borrow();
        !s.disposed
            && s.iframe.as_ref().is_some_and(|f| f.is_connected())
            && s.same_route(&with, &allow)
    });
    let host_for_task = host.clone();
    spawn_local(async move {
        if !reuse {
            render_in_iframe(&host_for_task, &cell, with, allow);
        } else if let Some(state) = cell.borrow().as_ref() {
            crate::bridge::refresh_context(&host_for_task, state);
        }
    });
}

/// Whether `host` has an origin to render `with` on: the deployment names
/// one for sites, and `with` is a site that gets one.
fn on_own_origin(host: &Element, with: &Location) -> bool {
    site_pattern(host).is_some_and(|pattern| site_origin(with, &pattern).is_some())
}

/// This element's site entity (`site:<uuid>`), minted once and stored on the
/// element's `data-site` attribute so re-resolves reuse it.
fn site_entity(host: &HtmlElement) -> String {
    if let Some(existing) = host.get_attribute("data-site").filter(|s| !s.is_empty()) {
        return existing;
    }
    let site = format!("site:{}", random_uuid());
    let _ = host.set_attribute("data-site", &site);
    site
}

/// A random uuid via `crypto.randomUUID()` (reflected so no extra web-sys
/// feature). Falls back to a timestamp-derived id if unavailable.
fn random_uuid() -> String {
    use wasm_bindgen::JsValue;
    use web_sys::js_sys::{Function, Reflect};
    (|| {
        let win = window()?;
        let crypto = Reflect::get(&win, &JsValue::from_str("crypto")).ok()?;
        let f = Reflect::get(&crypto, &JsValue::from_str("randomUUID"))
            .ok()?
            .dyn_into::<Function>()
            .ok()?;
        f.call0(&crypto).ok()?.as_string()
    })()
    .unwrap_or_else(|| format!("{:x}", web_sys::js_sys::Date::now() as u64))
}

/// Size a routed site's iframe to the surrounding viewport.
fn style_site_iframe(iframe: &HtmlIFrameElement) {
    let style = iframe.style();
    // `<tonk-site>` itself is `display: contents` (a transparent routing
    // element), so the iframe sizes against the surrounding layout, not the
    // element. `100dvh`/`100%` are viewport-/parent-relative so the iframe
    // fills regardless of nesting (top-level body child, or a flex slot in a
    // space chrome) instead of collapsing to the iframe's intrinsic ~150px.
    let _ = style.set_property("width", "100%");
    let _ = style.set_property("height", "100dvh");
    let _ = style.set_property("flex", "1 1 auto");
    let _ = style.set_property("align-self", "stretch");
    let _ = style.set_property("border", "0");
    let _ = style.set_property("display", "block");
    // The element is appended before its srcdoc is assigned, and a runtime
    // guest receives its theme only after the bridge handshake. Paint behind
    // both document states so a nested site cannot expose the browser's white
    // iframe canvas while its dark guest is starting.
    let _ = style.set_property(
        "background-color",
        "var(--wa-color-surface-default, light-dark(#e8e6e4, #161313))",
    );
}

/// The address `host` shows in its site: its `path` attribute as an address,
/// the site's root when it has none.
fn site_path(host: &Element) -> String {
    host.get_attribute("path").unwrap_or_default()
}

/// Frame the site `with` names, at the address `host`'s path names on the
/// site's own origin, and register the frame with the bridge. A prior frame
/// is torn down first, so a change of site replaces it.
///
/// The frame brings itself up: the site's worker answers the address with
/// the site's shell (or with whatever content the site keeps there), and
/// the shell loads the runtime and claims the route. This element gives it
/// nothing but a port, over which the two exchange the messages a frame
/// cannot act on alone, and reads back how far along it is.
fn render_in_iframe(host: &HtmlElement, cell: &StateCell, with: Location, allow: Allow) {
    teardown(cell);
    let element: Element = host.clone().into();
    let Some(pattern) = site_pattern(&element) else {
        return;
    };
    let Some(origin) = site_origin(&with, &pattern) else {
        return;
    };
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Some(iframe) = document
        .create_element("iframe")
        .ok()
        .and_then(|iframe| iframe.dyn_into::<HtmlIFrameElement>().ok())
    else {
        return;
    };
    let _ = iframe.set_attribute("sandbox", SANDBOX);
    // Permissions Policy, not a sandbox grant: without it the clipboard is
    // refused outright, whatever the sandbox allows.
    let _ = iframe.set_attribute("allow", "clipboard-write");
    style_site_iframe(&iframe);

    let state = Rc::new(RefCell::new(PortalState::new()));
    let profile = with.profile();
    state.borrow_mut().set_route(Some(with), allow);
    bridge::register_portal(&iframe, &element, &state);
    let _ = host.append_child(&iframe);
    if profile {
        expose_profile_worker(&iframe, &origin);
    }
    state.borrow_mut().iframe = Some(iframe.clone());
    *cell.borrow_mut() = Some(state.clone());
    load_site(&element, &iframe, &state, origin, Some(pattern));
}

/// Load the site's address in `iframe`, and watch for its shell.
fn load_site(
    host: &Element,
    iframe: &HtmlIFrameElement,
    state: &Rc<RefCell<PortalState>>,
    origin: String,
    pattern: Option<String>,
) {
    let _ = host.remove_attribute("data-ready");
    let address = site_url(&origin, &site_path(host));
    state.borrow_mut().set_origin(origin, pattern);
    let load = state.borrow().shell.get().load;
    watch_shell(host, iframe, state, load);
    let _ = iframe.set_attribute("src", &address);
}

/// Load the site's frame again, at its current address: what "try again"
/// does for a site that could not load.
pub(crate) fn reload_site(host: &Element, state: &Rc<RefCell<PortalState>>) {
    bridge::disconnect_task(state);
    let (iframe, origin, pattern) = {
        let mut s = state.borrow_mut();
        s.sever();
        let (Some(iframe), Some(origin)) = (s.iframe.clone(), s.origin().map(str::to_owned)) else {
            return;
        };
        (iframe, origin, s.site_pattern.clone())
    };
    load_site(host, &iframe, state, origin, pattern);
}

/// Register `<tonk-site>`. Idempotent. Installs the page-level `hello` /
/// runtime-injection message listener (the same one `<tonk-portal>` installs),
/// since `<tonk-site>` owns sealed iframes that hand-shake and request the
/// element runtime through it.
pub fn register() {
    crate::bridge::install_message_listener();
    crate::space_origin::install_reach();
    if let Some(win) = window()
        && win.custom_elements().get("tonk-site").is_undefined()
    {
        TonkSite::define("tonk-site");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> web_sys::Document {
        window().expect("window").document().expect("document")
    }

    fn site(attributes: &[(&str, &str)]) -> HtmlElement {
        register();
        let host: HtmlElement = document()
            .create_element("tonk-site")
            .expect("site")
            .dyn_into()
            .expect("html element");
        for (name, value) in attributes {
            host.set_attribute(name, value).expect("attribute");
        }
        document()
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach");
        host
    }

    #[dialog_common::test]
    fn it_says_so_when_the_deployment_names_no_origin_for_sites() {
        let host = site(&[("with", "main@did:key:zSpace"), ("allow", "self")]);

        assert_eq!(
            host.get_attribute("data-state").as_deref(),
            Some("malformed")
        );
        assert!(
            host.text_content()
                .unwrap_or_default()
                .contains("names no origin"),
            "a site with nowhere to render must say why"
        );
        assert!(
            host.query_selector("iframe").unwrap().is_none(),
            "and must not fall back to a sealed frame"
        );
        host.remove();
    }

    #[dialog_common::test]
    fn it_paints_the_site_surface_before_the_guest_document_loads() {
        let iframe = document()
            .create_element("iframe")
            .expect("iframe")
            .dyn_into::<HtmlIFrameElement>()
            .expect("iframe cast");

        style_site_iframe(&iframe);

        let background = iframe
            .style()
            .get_property_value("background-color")
            .expect("background-color property");
        assert!(
            background.contains("--wa-color-surface-default"),
            "the visible iframe must use the parent theme while its document loads; got: {background:?}"
        );
    }
}
