//! Guest-side host installation for the sealed iframe.
//!
//! The guest runs the REAL host IO surface (`tonk_host::install_io`):
//! consumer events bubble to the guest `document`, where the host's
//! own listeners service them over plain `fetch`/SSE. Inside the
//! sealed (opaque-origin) iframe, `window.fetch` is the portal
//! bootstrap's override, which has the outer frame perform each
//! request and stream the response back — so the same transport code
//! serves the top document and every guest, and there is no
//! per-operation envelope relay to miss an event. `window.tonk` is
//! sugar for app code, not the elements' transport.
//!
//! What remains guest-specific here:
//!
//! - the navigation click relay — the opaque guest cannot move the
//!   parent, so in-app link clicks post their href over
//!   `window.tonk.navigate` for the host to perform;
//! - binding the per-tab `site` entity onto opted-in descendants once
//!   the bridge context arrives.

use std::cell::RefCell;

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, window};

/// An installed listener, kept alive for the page's lifetime.
type Listener = Closure<dyn FnMut(Event)>;

thread_local! {
    /// The installed navigation listener. `Some` once [`install`] ran.
    static INSTALLED: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

/// Install the guest host: the real IO surface plus the navigation
/// click relay. Idempotent. Also binds the per-tab `site` entity onto
/// opted-in descendants once the bridge context arrives.
pub fn install() {
    // The real host's operation listeners + `with` observer, minus the
    // top-page-only effects (idle-sync heartbeat, navigate provider).
    tonk_host::install_io();

    let already = INSTALLED.with(|cell| cell.borrow().is_some());
    if already {
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };

    // Navigation relay: a link click inside the opaque guest can't move
    // the parent, so catch it at the document and post the href over the
    // bridge for the host to perform. Capture phase so it runs before any
    // app handler and before the (blocked-anyway) native navigation.
    let nav = make_nav_listener();
    let _ = document.add_event_listener_with_callback_and_bool(
        "click",
        nav.as_ref().unchecked_ref(),
        true,
    );
    INSTALLED.with(|cell| *cell.borrow_mut() = Some(nav));

    // Bind the per-tab `site` entity: any element that opted in with
    // `data-tonk-entity="site"` gets its `entity` set to the host's site
    // (the X-Tonk-Site the SW keys this tab's `tonk:site` facts on —
    // `window.tonk.context.site`). That is how the routing shell
    // (`<tonk-display model=tonk:site data-tonk-entity="site">`) resolves
    // its own tab's location/route. Deferred until `window.tonk.ready`
    // resolves, since the context (with the site) arrives asynchronously
    // after the host's ready envelope.
    spawn_local(async move {
        await_tonk_ready().await;
        fill_site_entities(&document);
    });
}

/// Build the document click listener that relays in-guest link navigation
/// to the host over `window.tonk.navigate`.
fn make_nav_listener() -> Listener {
    Closure::wrap(Box::new(move |event: Event| {
        // Leave modified clicks (new tab/window, middle-click) to the
        // browser — though in the sandbox they're inert, honoring them keeps
        // behavior predictable.
        if let Ok(mouse) = event.clone().dyn_into::<web_sys::MouseEvent>()
            && (mouse.meta_key() || mouse.ctrl_key() || mouse.shift_key() || mouse.button() != 0)
        {
            return;
        }
        let Some(href) = event.target().and_then(closest_anchor_href) else {
            return;
        };
        // Only relay in-app navigations: a path (`/…`) or a same-document
        // href. Skip fragments, mailto:, external schemes, etc.
        if !href.starts_with('/') || href.starts_with("//") {
            return;
        }
        event.prevent_default();
        if let Some(tonk) = window_tonk()
            && let Some(navigate) = get_fn(&tonk, "navigate")
        {
            let _ = navigate.call1(&tonk, &JsValue::from_str(&href));
        }
    }) as Box<dyn FnMut(Event)>)
}

/// Walk up from an event target to the nearest `<a>` and read its `href`
/// attribute (the raw attribute, not the resolved `.href` which an opaque
/// origin mangles to `null/…`).
fn closest_anchor_href(target: web_sys::EventTarget) -> Option<String> {
    let element = target.dyn_into::<web_sys::Element>().ok()?;
    let anchor = element.closest("a[href]").ok()??;
    anchor.get_attribute("href").filter(|h| !h.is_empty())
}

/// `window.tonk` if the portal bootstrap installed it.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
}

/// The host's per-tab `site` entity (`window.tonk.context.site`), the
/// `X-Tonk-Site` the SW keys this tab's `tonk:site` facts on. Lives on `context`
/// (not `tonk` directly) because it is the HOST's id, delivered with the rest of
/// the context in the `ready` envelope.
fn site_id() -> Option<String> {
    let tonk: JsValue = window_tonk()?.into();
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    Reflect::get(&context, &JsValue::from_str("site"))
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

/// Await `window.tonk.ready` (resolves once the host's `ready` envelope, with
/// the context, has arrived). Resolves immediately if the bridge is absent.
async fn await_tonk_ready() {
    let Some(tonk) = window_tonk() else {
        return;
    };
    let Ok(ready) = Reflect::get(&tonk, &JsValue::from_str("ready")) else {
        return;
    };
    if let Ok(promise) = ready.dyn_into::<Promise>() {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// Set `entity` to the host's `site` entity on every element in the document
/// that opted in with `data-tonk-entity="site"`. Idempotent: `<tonk-display>`
/// observes `entity`, so a re-set after upgrade just re-resolves to the same
/// value.
fn fill_site_entities(document: &Document) {
    let Some(site) = site_id() else {
        return;
    };
    let Ok(matches) = document.query_selector_all("[data-tonk-entity=\"site\"]") else {
        return;
    };
    for i in 0..matches.length() {
        if let Some(el) = matches.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
            let _ = el.set_attribute("entity", &site);
        }
    }
}

/// Read `tonk[name]` as a Function.
fn get_fn(tonk: &Object, name: &str) -> Option<Function> {
    Reflect::get(tonk, &JsValue::from_str(name))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
}
