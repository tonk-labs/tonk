//! Depth annotator — helper consumer elements install in their
//! `connected_callback` so the host learns each subscription's
//! structural nesting at dispatch time.
//!
//! Every consumer-style element (one that mounts other consumers
//! via templates or iteration) installs a bubble-phase listener
//! on each operation event that increments `event.detail.depth`.
//! By the time the event reaches the host, `detail.depth` is the
//! number of consumer ancestors between the dispatcher and the
//! host.
//!
//! A bubbling event dispatched on an element does not trigger
//! that element's own bubble listener (bubble starts at
//! `event.target.parentNode`). So a dispatcher does not count
//! itself; `depth` ends up as the number of *strict* consumer
//! ancestors. That is the structural distance.

use js_sys::{Number, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{CustomEvent, Element};

use crate::events;

/// One named listener: the event name plus the closure backing it.
type NamedListener = (&'static str, Closure<dyn FnMut(CustomEvent)>);

/// Handle returned by `install_depth_annotator`. Hold it for the
/// element's lifetime so the listener stays attached; drop it
/// to detach.
pub struct DepthAnnotator {
    host: Element,
    closures: Vec<NamedListener>,
}

impl Drop for DepthAnnotator {
    fn drop(&mut self) {
        for (name, closure) in &self.closures {
            let _ = self
                .host
                .remove_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        }
    }
}

/// Install a bubble-phase depth-annotator listener on `host` for
/// each operation event. Returns a handle whose `Drop` detaches
/// the listeners.
///
/// Call this from each consumer element's `connected_callback`
/// and store the returned handle in the element's per-instance
/// state. The handle's `Drop` is invoked on `disconnected_callback`
/// (when the state is taken / cleared).
pub fn install_depth_annotator(host: &Element) -> DepthAnnotator {
    let mut closures = Vec::with_capacity(events::OPERATIONS.len());
    for &name in events::OPERATIONS {
        let closure = Closure::wrap(Box::new(move |ev: CustomEvent| {
            increment(&ev);
        }) as Box<dyn FnMut(CustomEvent)>);
        let _ = host.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        closures.push((name, closure));
    }
    DepthAnnotator {
        host: host.clone(),
        closures,
    }
}

/// Read `event.detail.depth`, default 0, add 1, write back.
fn increment(ev: &CustomEvent) {
    let detail = ev.detail();
    if !detail.is_object() {
        return;
    }
    let obj = detail.unchecked_into::<Object>();
    let key = JsValue::from_str("depth");
    let current = Reflect::get(&obj, &key)
        .ok()
        .and_then(|v| Number::from(v).as_f64())
        .unwrap_or(0.0);
    let next = JsValue::from_f64(current + 1.0);
    let _ = Reflect::set(&obj, &key, &next);
}
