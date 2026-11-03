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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use custom_elements::CustomElement;
use tonk_host::consumer::{self, Subscription};
use tonk_host::location::{Allow, Location};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

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
    /// The self-heal subscription on this site's own `tonk:site` stamp.
    /// Held for the element's lifetime; its `Drop` cancels upstream.
    heal: Rc<RefCell<Option<Subscription>>>,
    /// Whether a resolve pass is already queued for this turn, and why.
    /// The renderer writes `with`, `allow` and `path` one `setAttribute` at
    /// a time; each fires a callback, and they all coalesce into the one
    /// queued pass, which reads the attributes as they stand once the burst
    /// is over.
    scheduled: Rc<Cell<Option<Trigger>>>,
    /// Whether this element has been connected before, so a later connect
    /// is counted as a remount rather than a first mount.
    connected_before: bool,
}

/// Why `<tonk-site>` is resolving its route, so a guest frame it builds is
/// counted under the cause that made it build one.
#[derive(Clone, Copy)]
enum Trigger {
    /// The element was connected for the first time.
    Mount,
    /// The element was connected again after being disconnected.
    Remount,
    /// `path`, `with` or `allow` changed on a connected element.
    Navigation,
}

impl Trigger {
    fn guest_boot(self) -> &'static str {
        use tonk_common::diagnostics::names;
        match self {
            Trigger::Mount => names::SITE_GUEST_BOOT_MOUNT,
            Trigger::Remount => names::SITE_GUEST_BOOT_REMOUNT,
            Trigger::Navigation => names::SITE_GUEST_BOOT_NAVIGATION,
        }
    }
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
        let trigger = if self.connected_before {
            Trigger::Remount
        } else {
            Trigger::Mount
        };
        self.connected_before = true;
        self.schedule(this, trigger);
        // Navigation is just a `path` attribute change, handled by
        // `attribute_changed_callback`. The element never reads `window.location`;
        // whoever mounts the top-level site (`ui.rs`) owns updating `path` on URL
        // change, so this element stays uniform for top-level and nested alike.
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        teardown(&self.inner);
        // Cancel the self-heal subscription (Drop cancels upstream), and drop
        // its probe with it: a probe left behind would read as "already
        // watching" when the element is reconnected, and the heal would
        // never subscribe again.
        self.heal.borrow_mut().take();
        if let Ok(Some(probe)) = this.query_selector(":scope > [data-site-heal]") {
            probe.remove();
        }
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
        // `resolve_and_render` (`is_connected` gate), and every callback in
        // one turn coalesces into a single pass (`schedule`).
        let first_set = old.as_deref().is_none_or(str::is_empty);
        let re_route = match name.as_str() {
            "path" => old != new,
            "with" | "allow" => !first_set && old != new,
            _ => false,
        };
        if re_route {
            self.schedule(this, Trigger::Navigation);
        }
    }
}

impl TonkSite {
    /// Queue one resolve pass for this turn, unless one is already queued.
    ///
    /// Deciding per callback was the bug this guards: a space-to-space
    /// switch writes `with`, then `allow`, then `path`, and each callback
    /// judged iframe reuse against the state from before any of them had
    /// run. None matched, so each built its own iframe and the next tore it
    /// down again: two or three guest boots for one navigation, each a blank
    /// frame and a spinner. One pass after the burst sees the final
    /// attributes and builds (or reuses) once.
    fn schedule(&self, this: &HtmlElement, trigger: Trigger) {
        // A connect in the burst names the pass: the attribute writes
        // around it are part of mounting, not a navigation.
        if let Some(queued) = self.scheduled.get() {
            if matches!(queued, Trigger::Navigation) {
                self.scheduled.set(Some(trigger));
            }
            return;
        }
        self.scheduled.set(Some(trigger));
        let host = this.clone();
        let cell = self.inner.clone();
        let heal = self.heal.clone();
        let scheduled = self.scheduled.clone();
        // Deferred off the callback: rendering touches the element's method
        // delegates and the bridge, and doing that inside a custom-element
        // callback re-enters the lock the `custom_elements` runtime holds
        // ("cannot recursively acquire mutex"). A microtask still runs
        // before the browser paints, so the burst never shows half-applied.
        spawn_local(async move {
            let _ =
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
                    .await;
            let trigger = scheduled.take().unwrap_or(Trigger::Navigation);
            resolve_and_render(&host, cell, trigger);
            install_self_heal(&host, heal);
        });
    }
}

/// Tear down the portal iframe held by `cell`, if any.
///
/// TWO-PHASE: sever the comms (aborts + port closes via `clear_subs`),
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
        s.clear_subs();
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
/// bring up the sealed iframe rendering the matched route. A missing or
/// malformed `with`/`allow` renders a visible error; a failed registration
/// leaves the element empty rather than throwing.
fn resolve_and_render(this: &HtmlElement, cell: StateCell, trigger: Trigger) {
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
    // This runs from the pass `TonkSite::schedule` queued, already off the
    // custom-element callback, so it can tear down and rebuild the portal
    // directly.
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
    if !reuse {
        tonk_common::diagnostics::count(trigger.guest_boot());
        render_in_iframe(&host, &cell, &site, with, allow);
    } else if let Some(state) = cell.borrow().as_ref() {
        tonk_common::diagnostics::count(tonk_common::diagnostics::names::SITE_REROUTE);
        crate::bridge::refresh_context(&host, state);
    }
    spawn_local(async move {
        if let Err(error) = tonk_host::consumer::claim(&host.clone().into(), &request).await {
            tonk_common::log!("tonk-site: load claim failed for {path}: {error:?}");
        }
    });
}

/// Install the self-heal subscription: watch this site's own `tonk:site`
/// stamp (the `path` field on the site entity) and, whenever a frame comes
/// back EMPTY, re-assert the `tonk:load` claim for the current `path`.
///
/// The stamp lives in the service worker's in-memory overlay: a stopped or
/// replaced worker loses it, and without this the reconnected site display
/// serves an empty frame forever — the never-ending spinner. Empty frame ⇒
/// "my stamp vanished" ⇒ re-claim; the fresh stamp re-renders the guest in
/// place. Data-driven, so it heals ANY overlay loss, and self-limiting: a
/// re-claim that lands produces a non-empty frame, and an unchanged empty
/// state pushes no further frames.
///
/// The subscription rides a hidden probe child (its own `reset` property),
/// not the site element itself — the portal's `reset` shim is the bridge
/// relay's. The probe carries a copy of the site's `with`, so every resolve
/// pass calls this again: a probe already watching the current `with` is
/// kept, and one left on a previous `with` (a space-to-space switch of a
/// nested site) is replaced, or it would go on healing the old branch while
/// the new one stays unstamped. A site that first connected with a
/// `{…}`-placeholder `with` gets its heal on the pass that resolves it.
/// Deferred a microtask: inside a render pass this callback can be
/// delivered after the element was detached again (the reaction-queue
/// hazard), and the dispatch must come from a connected tree.
fn install_self_heal(this: &HtmlElement, slot: Rc<RefCell<Option<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED))
            .await;
        if !host.is_connected() {
            return;
        }
        // The probe routes by the site's OWN `with` (routing is self-only, no
        // ancestor walk); without one it would subscribe to a bare `/query`, a
        // 404 the heal then retries forever. A `{…}` placeholder (unstamped
        // template) waits for the pass that resolves it.
        let Some(with) = host
            .get_attribute("with")
            .filter(|v| !v.is_empty() && !v.contains('{'))
        else {
            return;
        };
        let current = host
            .query_selector(":scope > [data-site-heal]")
            .ok()
            .flatten();
        if let Some(existing) = current {
            if existing.get_attribute("with").as_deref() == Some(with.as_str()) {
                return;
            }
            existing.remove();
            // Dropping the subscription cancels the old branch's watch.
            slot.borrow_mut().take();
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(probe) = document.create_element("span") else {
            return;
        };
        let _ = probe.set_attribute("hidden", "");
        let _ = probe.set_attribute("data-site-heal", "");
        let _ = probe.set_attribute("with", &with);
        let _ = host.append_child(&probe);

        let site = site_entity(&host);
        let host_for_frame = host.clone();
        let reset: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                if js_sys::Array::from(&payload).length() == 0 {
                    heal_claim(&host_for_frame);
                }
            }));
        let _ = js_sys::Reflect::set(&probe, &"reset".into(), reset.as_ref());
        reset.forget();

        let body = match heal_query(&site) {
            Some(body) => body,
            None => return,
        };
        let tag = JsValue::from_str("site-heal");
        match consumer::subscribe_claimed(&probe, &body, Some(&tag)).await {
            // A probe replaced while its subscribe was in flight drops the
            // late subscription rather than overwrite its successor's.
            Ok(subscription) if probe.is_connected() => *slot.borrow_mut() = Some(subscription),
            Ok(_) => {}
            Err(error) => {
                tonk_common::log!("tonk-site: heal subscribe failed: {error:?}");
            }
        }
    });
}

/// Re-assert the `tonk:load` claim for the site's CURRENT path — the heal
/// action when the stamp has vanished. Skips an unresolved `{…}` path (the
/// stamped frame re-runs the resolve, which claims itself).
fn heal_claim(host: &Element) {
    let path = host
        .get_attribute("path")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_owned());
    if path.contains('{') {
        return;
    }
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let Ok(html_host) = host.clone().dyn_into::<HtmlElement>() else {
        return;
    };
    let site = site_entity(&html_host);
    let request = load_claim(&site, &path);
    let host = host.clone();
    spawn_local(async move {
        if let Err(error) = consumer::claim(&host, &request).await {
            tonk_common::log!("tonk-site: heal claim failed for {path}: {error:?}");
        }
    });
}

/// The heal subscription's body: the site entity's stamped `path` — one
/// cardinality-one field, so an empty frame is exactly "no stamp".
fn heal_query(site: &str) -> Option<JsValue> {
    let body = format!(
        r#"{{"predicate":{{"with":{{"path":{{"the":"xyz.tonk.site/path","as":"Text","cardinality":"one"}}}}}},"terms":{{"this":{site:?},"path":{{"?":{{"name":"path"}}}}}}}}"#
    );
    js_sys::JSON::parse(&body).ok()
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
    connect_portal(host, cell.as_ref(), Some(with), allow, style_site_iframe);
}

/// Register `<tonk-site>`. Idempotent. Installs the page-level `hello` /
/// runtime-injection message listener (the same one `<tonk-portal>` installs),
/// since `<tonk-site>` owns sealed iframes that hand-shake and request the
/// element runtime through it.
pub fn register() {
    // Expose this realm's counters as `globalThis.__tonkDiagnostics`.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk_common::diagnostics::install_global();
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Promise, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::CustomEvent;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> web_sys::Document {
        window().expect("window").document().expect("document")
    }

    /// Yield a few microtasks so deferred installs and spawned claims run.
    async fn flush() {
        for _ in 0..6 {
            let _ =
                wasm_bindgen_futures::JsFuture::from(Promise::resolve(&JsValue::UNDEFINED)).await;
        }
    }

    /// A minimal stand-in host: claims `tonk-subscribe` (recording the tag)
    /// and `tonk-claim` (recording the request), enough for the heal loop.
    struct FakeHost {
        container: Element,
        sub_tag: Rc<RefCell<Option<String>>>,
        claim_body: Rc<RefCell<Option<String>>>,
        _listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
    }

    impl FakeHost {
        fn install() -> Self {
            let container = document().create_element("div").expect("div");
            document()
                .body()
                .expect("body")
                .append_child(&container)
                .expect("attach");
            let sub_tag = Rc::new(RefCell::new(None));
            let claim_body = Rc::new(RefCell::new(None));
            let mut listeners = Vec::new();
            {
                let sub_tag = sub_tag.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        *sub_tag.borrow_mut() = Reflect::get(&detail, &"tag".into())
                            .ok()
                            .and_then(|t| t.as_string());
                        let sub = Object::new();
                        let cancel: Closure<dyn FnMut()> =
                            Closure::wrap(Box::new(move || {}) as Box<dyn FnMut()>);
                        let cancel_fn: Function = cancel.into_js_value().unchecked_into();
                        let _ = Reflect::set(&sub, &"cancel".into(), &cancel_fn);
                        let _ = Reflect::set(&detail, &"subscription".into(), &sub);
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container.add_event_listener_with_callback(
                    "tonk-subscribe",
                    cb.as_ref().unchecked_ref(),
                );
                listeners.push(cb);
            }
            {
                let claim_body = claim_body.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let request = Reflect::get(&detail, &"request".into()).unwrap();
                        *claim_body.borrow_mut() =
                            js_sys::JSON::stringify(&request).ok().map(String::from);
                        let _ = Reflect::set(
                            &detail,
                            &"result".into(),
                            &Promise::resolve(&JsValue::from_str("ok")),
                        );
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-claim", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            Self {
                container,
                sub_tag,
                claim_body,
                _listeners: listeners,
            }
        }
    }

    /// The heal loop: an EMPTY frame on the site's own stamp subscription
    /// re-asserts the `tonk:load` claim for the current path (the stamp
    /// lives in the SW's in-memory overlay, so a worker restart loses it);
    /// a non-empty frame claims nothing.
    #[dialog_common::test]
    async fn it_reclaims_the_site_when_the_stamp_vanishes() {
        let fake = FakeHost::install();
        let host: HtmlElement = document()
            .create_element("tonk-site")
            .expect("site")
            .dyn_into()
            .expect("html element");
        let _ = host.set_attribute("path", "/space/x");
        let _ = host.set_attribute("data-site", "site:test-heal");
        // The site carries its own `with`; the heal probe copies it so its
        // subscription routes to the site's branch (self-only routing).
        let _ = host.set_attribute("with", "main@did:key:zSpace");
        fake.container.append_child(&host).expect("attach site");

        let slot = Rc::new(RefCell::new(None));
        install_self_heal(&host, slot.clone());
        flush().await;
        assert_eq!(
            fake.sub_tag.borrow().as_deref(),
            Some("site-heal"),
            "the heal subscription opens once the site connects"
        );
        assert!(slot.borrow().is_some());

        let probe = host
            .query_selector("[data-site-heal]")
            .expect("query")
            .expect("probe mounted");
        let reset: Function = Reflect::get(&probe, &"reset".into())
            .expect("reset prop")
            .dyn_into()
            .expect("reset fn");

        // A non-empty frame is a live stamp: no claim.
        let full = Array::of1(&Object::new());
        let _ = reset.call2(&probe, &full, &JsValue::UNDEFINED);
        flush().await;
        assert!(
            fake.claim_body.borrow().is_none(),
            "a live stamp must not re-claim"
        );

        // An empty frame is a vanished stamp: re-claim the current path.
        let _ = reset.call2(&probe, &Array::new().into(), &JsValue::UNDEFINED);
        flush().await;
        let claim = fake.claim_body.borrow().clone().expect("claim dispatched");
        assert!(
            claim.contains("/space/x") && claim.contains("site:test-heal"),
            "the re-claim carries the current path and site entity, got: {claim}"
        );
    }

    /// Count the iframes added under `site`, through a JS `MutationObserver`
    /// (its records arrive as microtasks, which `flush` drains).
    fn count_iframe_builds(site: &HtmlElement) -> Object {
        let observe = Function::new_with_args(
            "site",
            "const box = {n: 0};\
             new MutationObserver((records) => {\
               for (const record of records)\
                 for (const node of record.addedNodes)\
                   if (node.tagName === 'IFRAME') box.n += 1;\
             }).observe(site, {childList: true});\
             return box;",
        );
        observe
            .call1(&JsValue::NULL, site)
            .expect("observer installed")
            .dyn_into()
            .expect("counter object")
    }

    fn builds(counter: &Object) -> f64 {
        Reflect::get(counter, &"n".into())
            .ok()
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0)
    }

    /// A space-to-space switch writes `with`, `allow` and `path` one
    /// attribute at a time, all in one turn. The site must boot ONE guest
    /// for it: deciding reuse per attribute judged each write against the
    /// state from before any of them ran, and booted two or three guests,
    /// each tearing down the one before (a blank frame and the pulsing dot
    /// for every extra boot).
    #[dialog_common::test]
    async fn it_boots_one_guest_for_a_space_switch() {
        register();
        let fake = FakeHost::install();
        let host: HtmlElement = document()
            .create_element("tonk-site")
            .expect("site")
            .dyn_into()
            .expect("html element");
        let _ = host.set_attribute("with", "main@did:key:zSpaceA");
        let _ = host.set_attribute("allow", "main@did:key:zSpaceA");
        let counter = count_iframe_builds(&host);
        fake.container.append_child(&host).expect("attach site");
        flush().await;
        assert_eq!(builds(&counter), 1.0, "connecting boots the first guest");

        let _ = host.set_attribute("with", "main@did:key:zSpaceB");
        let _ = host.set_attribute("allow", "main@did:key:zSpaceB");
        let _ = host.set_attribute("path", "/inspector");
        flush().await;
        assert_eq!(
            builds(&counter),
            2.0,
            "one space switch boots exactly one more guest"
        );

        // A pure path change keeps the live guest.
        let _ = host.set_attribute("path", "/");
        flush().await;
        assert_eq!(builds(&counter), 2.0, "a path change re-routes in place");

        host.remove();
        fake.container.remove();
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
            "the visible iframe must use the parent theme while about:blank and srcdoc bootstrap; got: {background:?}"
        );
    }
}
