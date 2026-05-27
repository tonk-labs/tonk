//! `<tonk-repository name="…">` — annotates `detail.space` on
//! outbound consumer events as they bubble.
//!
//! Passive annotator. No IO. Listens on the four operation
//! events in bubble phase; if `detail.space` is not already set,
//! writes its own `name` attribute. Inner-most-wins via `??=`
//! semantics.

use std::cell::RefCell;

use custom_elements::CustomElement;
use js_sys::{Object, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CustomEvent, Element, HtmlElement, window};

use crate::events;

/// One named listener: the event name plus the closure backing it.
pub(crate) type NamedListener = (&'static str, Closure<dyn FnMut(CustomEvent)>);

/// Outer per-element struct.
#[derive(Default)]
pub(crate) struct TonkRepository {
    listeners: RefCell<Vec<NamedListener>>,
}

impl CustomElement for TonkRepository {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["name"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let installed = attach_annotators(&host, "space");
        *self.listeners.borrow_mut() = installed;
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let listeners = std::mem::take(&mut *self.listeners.borrow_mut());
        for (name, closure) in &listeners {
            let _ =
                host.remove_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        dispatch_context_refresh(this);
    }
}

/// Attach one bubble-phase listener per operation event. Each
/// stamps `detail.<field>` with this element's `name` attribute
/// if not already set.
pub(crate) fn attach_annotators(host: &Element, field: &'static str) -> Vec<NamedListener> {
    let mut out = Vec::with_capacity(events::OPERATIONS.len());
    for &name in events::OPERATIONS {
        let host_for_handler = host.clone();
        let closure = Closure::wrap(Box::new(move |ev: CustomEvent| {
            annotate(&host_for_handler, field, &ev);
        }) as Box<dyn FnMut(CustomEvent)>);
        let _ = host.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        out.push((name, closure));
    }
    out
}

/// Write `host.getAttribute("name")` into `event.detail[field]`
/// if the field is not already set.
fn annotate(host: &Element, field: &str, ev: &CustomEvent) {
    let Some(name) = host.get_attribute("name").filter(|s| !s.is_empty()) else {
        return;
    };
    let detail = ev.detail();
    let obj = if detail.is_object() {
        detail.unchecked_into::<Object>()
    } else {
        // No detail object to annotate. Consumer dispatched without
        // a detail at all. Skip — operation listeners require a
        // detail and will fail loudly there.
        return;
    };
    let key = JsValue::from_str(field);
    let existing = Reflect::get(&obj, &key).unwrap_or(JsValue::UNDEFINED);
    if existing.is_undefined() || existing.is_null() {
        let _ = Reflect::set(&obj, &key, &JsValue::from_str(&name));
    }
}

/// Dispatch a bubbling `tonk-context-refresh` event on the
/// routing element so the host can orchestrate a refresh.
fn dispatch_context_refresh(this: &HtmlElement) {
    let Some(_win) = window() else { return };
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    let Ok(ev) = CustomEvent::new_with_event_init_dict(events::CONTEXT_REFRESH, &init) else {
        return;
    };
    let target: &Element = this.unchecked_ref();
    let _ = target.dispatch_event(&ev);
}

/// Register `<tonk-repository>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkRepository::define("tonk-repository");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-repository").is_undefined()
}
