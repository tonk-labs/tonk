//! `<tonk-site with="…" allow="…" path="…">` — the sealed routing element.
//!
//! `<tonk-site>` is a portal that routes: it registers a per-tab site for its
//! path on the branch its `with` attribute names, then renders the matched
//! route inside its own sealed iframe (reusing the portal machinery). It is
//! the recursive unit of the UI — the top page mounts one, a space's chrome
//! mounts a nested one for the repo, and so on; each owns one isolation
//! boundary.
//!
//! `with` (the context, `branch@repo`) and `allow` (the reach its guest may
//! ask for: `*`, `self`, or explicit locations) are **both required** — a
//! site missing or malforming either renders a visible error at connect.
//! There is no inheritance and no defaulting: every site is fully
//! self-describing, so privilege never leaks downward.
//!
//! Flow on connect (and on navigation):
//! 1. Parse `with` + `allow`; take the path from the `path` attribute (a
//!    nested router; the top page's mount keeps it synced to the location).
//! 2. Assert the transient `tonk:load` claim routed at `with` → the SW
//!    stamps the tab's `tonk:site` on that branch (matching the path
//!    against its `route!` table).
//! 3. Set the iframe `content` to the `tonk:site` display and bring up the
//!    sealed iframe via [`connect_portal`], passing `with`/`allow` so the
//!    bridge pins un-routed guest operations and enforces the reach. The
//!    guest's `<tonk-display>` subscribes to the site and renders the
//!    matched `{concept}`; a later route change restamps the site, and the
//!    subscription re-renders — downward propagation with no custom channel.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_host::location::{Allow, Location};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlElement, HtmlIFrameElement, window};

use crate::bridge::PortalState;
use crate::shared::{connect_portal, install_method_shims};

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
        // subscription re-renders. The initial values are handled by
        // `connected_callback`, so skip the first-set callback — NOTE the
        // `custom-elements` JS shim coerces a null `oldValue` to `""`, so a
        // first set arrives as `Some("")`, never `None`.
        let first_set = old.as_deref().is_none_or(str::is_empty);
        if matches!(name.as_str(), "path" | "with" | "allow") && !first_set && old != new {
            resolve_and_render(this, self.inner.clone());
        }
    }
}

/// Tear down the portal iframe held by `cell`, if any.
fn teardown(cell: &StateCell) {
    if let Some(state) = cell.borrow_mut().take() {
        let mut s = state.borrow_mut();
        s.disposed = true;
        s.clear_subs();
        if let Some(iframe) = s.iframe.take() {
            crate::bridge::unregister_portal(&iframe);
            if let Some(parent) = iframe.parent_node() {
                let _ = parent.remove_child(&iframe);
            }
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
/// bring up the sealed iframe rendering the matched route. A missing or
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
    // Clear a previous pass's visible error (a re-stamp can heal a
    // malformed site) so the error text never lingers next to the iframe.
    if host.get_attribute("data-state").as_deref() == Some("malformed") {
        host.set_text_content(None);
        let _ = host.remove_attribute("data-state");
    }
    // `<tonk-site>` routes the `path` attribute it is given, defaulting to the
    // branch root (`/`) when absent. It NEVER reads `window.location` itself —
    // the top-level mount is given the document path explicitly (by `ui.rs`, which
    // reads the location + listens for navigation and sets `path`), so the element
    // is uniform: top-level and nested both just route their `path`.
    let mut path = host
        .get_attribute("path")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_owned());

    // A `{…}` left in the path is an unresolved template (a partial substitution
    // mid-render before the real frame lands). Skip; the real frame re-sets
    // `path` and re-runs this.
    if path.contains('{') {
        return;
    }

    // The route table matches `/`-prefixed paths. A `{*rest}` span captured by a
    // route has NO leading slash (`/space/{id}/inspector` → `"inspector"`), so
    // prefix one or `match_route` finds nothing and the site stays unstamped.
    if !path.starts_with('/') {
        path = format!("/{path}");
    }
    let path = path;

    // The site entity this element renders against. Minted ONCE (reused across
    // re-resolves/navigations) so a navigation re-asserts `tonk:load` against the
    // same entity — the cardinality-one `tonk:site` fields supersede in place and
    // the live subscription re-renders, no teardown. Per-element, so two
    // `<tonk-site>`s on one page never share a site entity (even on one branch).
    let site = site_entity(&host);

    // Register the site by asserting a transient `tonk:load { this: site, path }`
    // through the regular transact API. The claim dispatches bare on this
    // element, so it resolves against the site's own `with` attribute — at
    // the top page via the installed host, in a guest via the relay's
    // forwarded route (judged by the enclosing portal's `allow`). The SW's
    // `LoadHandler` matches `path` against the route table and stamps
    // `tonk:site` onto `site`.
    let request = load_claim(&site, &path);
    // Defer BOTH the claim and the iframe bring-up off this turn. `resolve_and_render`
    // runs synchronously inside `connected_callback` / `attribute_changed_callback`,
    // and `render_in_iframe` tears down + rebuilds the portal (touching the element's
    // method delegates and the bridge). Doing that synchronously inside a custom-
    // element callback re-enters the single-threaded lock the `custom_elements`
    // runtime holds across the callback → "cannot recursively acquire mutex". A
    // `spawn_local` lets the callback return first, so the render happens on a clean
    // stack.
    //
    // Bring the iframe up FIRST, then fire the load claim WITHOUT awaiting it. The
    // iframe boot (wasm + Web Awesome + chrome upgrade) has no dependency on the
    // `tonk:site` stamp landing first: the guest's `<tonk-display model=tonk:site>`
    // SUBSCRIBES to the site entity, so whenever the stamp lands the subscription
    // delivers the frame and the content renders. Awaiting the claim's `/transact`
    // round-trip before the bring-up serialized the two — the iframe sat idle for
    // the whole round-trip before it even started booting — so overlap them.
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
            render_in_iframe(&host_for_task, &cell, &site, with, allow);
        }
        if let Err(error) =
            tonk_host::consumer::claim(&host_for_task.clone().into(), &request).await
        {
            tonk_common::log!("tonk-site: load claim failed for {path}: {error:?}");
        }
    });
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

/// Build the transient `tonk:load { this: site, path }` transact body the
/// consumer `claim` API takes. The predicate carries the `tonk:load` descriptor
/// INLINE (`kind: transient` + the one `xyz.tonk.site/path` field), so the worker
/// resolves against the inline shape — no dependency on `tonk:load` being
/// resolvable by name on the branch. Matches the wire shape
/// `tonk-display`'s event delegate posts (`{ claims: [ { op, application } ] }`).
fn load_claim(site: &str, path: &str) -> wasm_bindgen::JsValue {
    use web_sys::js_sys::JSON;
    let body = format!(
        r#"{{"claims":[{{"op":"assert","application":{{"predicate":{{"kind":"transient","concept":{{"with":{{"path":{{"the":"xyz.tonk.site/path","as":"Text","cardinality":"one"}}}}}}}},"parameters":{{"this":{site:?},"path":{path:?}}}}}}}]}}"#
    );
    JSON::parse(&body).unwrap_or(wasm_bindgen::JsValue::NULL)
}

/// Build the guest content (the `tonk:site` display) and bring up the sealed
/// iframe via [`connect_portal`]. The iframe always renders in `runtime` mode
/// (the guest needs our element runtime). If an iframe already exists (a
/// re-resolve), tear it down first so the new content replaces it.
///
/// The guest content carries no routing context of its own: the guest relay
/// forwards its `<tonk-display>`'s queries up to the parent, where the bridge
/// pins them to this site's `with`. So the `tonk:site` display resolves
/// against exactly the branch the site was stamped on.
fn render_in_iframe(
    host: &HtmlElement,
    cell: &StateCell,
    site: &str,
    with: Location,
    allow: Allow,
) {
    // The display carries slotted placeholders for the pre-stamp window so it
    // shows a quiet spinner instead of flashing its loud `no-entity`
    // concept-mismatch dump while the SW is still stamping `tonk:site`.
    let content = crate::site_content::guest_content(site);

    // Tear down any prior iframe so a re-resolve (navigation) replaces it.
    teardown(cell);

    // The iframe needs `content` + `runtime` to drive `connect_portal`.
    let _ = host.set_attribute("content", &content);
    let _ = host.set_attribute("runtime", "");

    // The site's parsed `with`/`allow` become the bridge's routing context
    // and reach: un-routed guest operations are pinned to `with`, and a
    // forwarded route is honored only if `allow` permits it (typed denial
    // otherwise).
    connect_portal(
        host,
        cell.as_ref(),
        Some(with),
        allow,
        |iframe: &HtmlIFrameElement| {
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
        },
    );
}

/// Register `<tonk-site>`. Idempotent. Installs the page-level `hello` /
/// runtime-injection message listener (the same one `<tonk-portal>` installs),
/// since `<tonk-site>` owns sealed iframes that hand-shake and request the
/// element runtime through it.
pub fn register() {
    crate::bridge::install_message_listener();
    if let Some(win) = window()
        && win.custom_elements().get("tonk-site").is_undefined()
    {
        TonkSite::define("tonk-site");
        // Install the `reset` / `update` / `error` prototype shims that route
        // a host subscription's frames into this portal's per-instance
        // delegates (and thus out to the sealed guest as `subscribe-event`).
        // Without these the host calls `consumer.reset(...)` on a `<tonk-site>`
        // that has no such method and the frame is silently dropped.
        install_method_shims("tonk-site");
    }
}
