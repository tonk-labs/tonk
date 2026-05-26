//! Host-level event delegation.
//!
//! A `Delegate` owns per-event-type JS listeners installed on the
//! `<tonk-display>` host element. On fire, it walks up from
//! `event.target` to the closest `[data-on<event>]`-bearing
//! ancestor, looks up that attribute's value (the concept name),
//! resolves the cached descriptor, builds a `TransactRequest`
//! body via [`super::extract::build_transact_body`], and POSTs to
//! `/api/repository/{repo}/branch/{branch}/transact`.
//!
//! Listeners stay attached for the lifetime of the host element.
//! When the host's children re-render incrementally (existing
//! tonk-display behaviour), the delegation listener keeps working
//! because it lives on the host, not on the buttons.
//!
//! Descriptors for every distinct concept-name in the template
//! are resolved up-front at mount time, so the click handler is
//! synchronous — no async hop to fetch a schema on each click.

use std::collections::HashMap;
use std::rc::Rc;

use tonk_concept::fetch::transact_post;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event};

use super::extract::build_transact_body;

/// Per-listener pair: the event-type name and the JS-side closure
/// whose lifetime owns its memory.
type ListenerEntry = (String, Closure<dyn FnMut(Event)>);

/// One installed delegation listener on the host, paired with the
/// `Closure` that owns its JS-side memory.
pub struct Delegate {
    /// The host the listeners are attached to. We need it on
    /// drop to remove the listeners.
    host: Element,
    /// One `(event_type, closure)` per registered event type.
    /// Dropped on `Delegate::drop`, which also calls
    /// `removeEventListener`.
    listeners: Vec<ListenerEntry>,
}

impl Delegate {
    /// Install delegation listeners on `host` for every event
    /// type in `event_types`. `descriptors` maps a concept name to
    /// its resolved descriptor JSON. `transact_url` is the
    /// `/transact` endpoint to POST claims to.
    ///
    /// Returns a `Delegate` value whose `Drop` impl removes the
    /// listeners. Store it on the renderer's state so listeners
    /// outlive the renderer-managed children.
    pub fn install(
        host: Element,
        event_types: impl IntoIterator<Item = String>,
        descriptors: HashMap<String, String>,
        transact_url: String,
    ) -> Self {
        let descriptors = Rc::new(descriptors);
        let url = Rc::new(transact_url);
        let mut listeners: Vec<ListenerEntry> = Vec::new();

        for event_type in event_types {
            let descriptors = Rc::clone(&descriptors);
            let url = Rc::clone(&url);
            let attr_name = format!("data-on{event_type}");
            let closure = Closure::wrap(Box::new(move |event: Event| {
                handle_event(&event, &attr_name, descriptors.as_ref(), url.as_ref());
            }) as Box<dyn FnMut(Event)>);
            let _ = host
                .add_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
            listeners.push((event_type, closure));
        }

        Self { host, listeners }
    }
}

impl Drop for Delegate {
    fn drop(&mut self) {
        for (event_type, closure) in self.listeners.drain(..) {
            let _ = self
                .host
                .remove_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
            // `closure` is moved out and dropped here; the
            // JS-side wrapper releases its references.
        }
    }
}

/// One event fire. Walk up to find the binding, look up the
/// descriptor, build the body, POST. Side effects (action
/// attributes) are applied inside `build_transact_body` during
/// the descriptor walk, so they happen before the POST kicks off
/// — which means `preventDefault` lands within the synchronous
/// handler tick, as required.
fn handle_event(
    event: &Event,
    attr_name: &str,
    descriptors: &HashMap<String, String>,
    transact_url: &str,
) {
    let Some(target) = event.target() else {
        return;
    };
    let Some(target_el) = target.dyn_ref::<Element>() else {
        return;
    };
    let selector = format!("[{attr_name}]");
    let Some(bound) = closest(target_el, &selector) else {
        return;
    };
    let Some(concept) = bound.get_attribute(attr_name) else {
        return;
    };
    let Some(descriptor_json) = descriptors.get(&concept) else {
        // Binding referenced a concept we couldn't resolve at
        // mount time. Silently ignore the click — the renderer
        // will already have logged the resolve failure.
        return;
    };
    // `dom:event` is the conventional `this:` for event-derived
    // transient assertions. Rules match on the asserted concept's
    // *fields*, not on `this`, so a fixed entity is fine — and
    // the assertion sweeps before the durable commit either way.
    let body = match build_transact_body(descriptor_json, &concept, EVENT_ENTITY, event) {
        Ok(b) => b,
        Err(e) => {
            log_error(format!("event handler: build body for {concept}: {e}"));
            return;
        }
    };
    let body_str = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => {
            log_error(format!("event handler: serialize body: {e}"));
            return;
        }
    };
    let url = transact_url.to_owned();
    spawn_local(async move {
        if let Err(e) = transact_post(&url, &body_str).await {
            log_error(format!("event handler: transact_post: {}", e.message));
        }
    });
}

/// `Element.closest(selector)` — walks up the parent chain until
/// it finds an element matching `selector`, or returns `None`.
fn closest(start: &Element, selector: &str) -> Option<Element> {
    start.closest(selector).ok().flatten()
}

/// Conventional `this:` for event-derived transient assertions.
/// Rules read these transients by their fields, not by `this`, so
/// a fixed entity is fine — and the assertion sweeps before the
/// durable commit either way.
const EVENT_ENTITY: &str = "dom:event";

fn log_error(message: String) {
    web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&message));
}
