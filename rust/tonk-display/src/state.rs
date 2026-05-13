//! Reflect lifecycle phase onto the host as `data-state`, and
//! surface error-state messages as a visible `<wa-callout>` inside
//! the host so users see what went wrong without needing to wire
//! the `tonk-display:error` event.

use wasm_bindgen::JsCast;
use web_sys::{Element, window};

/// The four states authors can target from CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Resolving concept / view / entity; no DOM yet.
    Loading,
    /// Entity rendered.
    Ready,
    /// Entity not found / stream emitted zero rows.
    Empty,
    /// Concept lookup, view lookup, or network failure.
    Error,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Loading => "loading",
            State::Ready => "ready",
            State::Empty => "empty",
            State::Error => "error",
        }
    }
}

/// Sentinel `data-` attribute we tag the injected callout with so
/// we can find and replace it on the next state transition without
/// disturbing whatever else the renderer mounted.
const ERROR_CALLOUT_ATTR: &str = "data-tonk-display-error";

/// Set `data-state` on `host`. Idempotent — safe to call repeatedly
/// with the same state.
///
/// Transitioning *away* from `Error` removes any callout we
/// previously injected.
pub fn set(host: &Element, state: State) {
    let _ = host.set_attribute("data-state", state.as_str());
    if state != State::Error {
        remove_error_callout(host);
    }
}

/// Transition the host to error state and surface a
/// `<wa-callout variant="danger">` inside the host with the given
/// `title` + `message`. The shape matches Web Awesome's reference
/// danger callout: icon in the `icon` slot, a bold title line, a
/// `<br>`, then the message body. Replaces any existing callout
/// from a prior error so the user always sees the most recent
/// failure.
pub fn set_error(host: &Element, title: &str, message: &str) {
    let _ = host.set_attribute("data-state", State::Error.as_str());
    remove_error_callout(host);
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(callout) = document.create_element("wa-callout") else {
        return;
    };
    let _ = callout.set_attribute("variant", "danger");
    let _ = callout.set_attribute(ERROR_CALLOUT_ATTR, "");

    // `<wa-icon slot="icon">` renders inside the callout's start
    // affordance so the surface reads as an alert at a glance.
    if let Ok(icon) = document.create_element("wa-icon") {
        let _ = icon.set_attribute("slot", "icon");
        let _ = icon.set_attribute("name", "circle-exclamation");
        let _ = callout.append_child(&icon);
    }
    // `<strong>` title line — short label naming the failure kind.
    if let Ok(strong) = document.create_element("strong") {
        strong.set_text_content(Some(title));
        let _ = callout.append_child(&strong);
    }
    // Line break between title and detail message, matching the WA
    // reference example.
    if let Ok(br) = document.create_element("br") {
        let _ = callout.append_child(&br);
    }
    let message_text = document.create_text_node(message);
    let _ = callout.append_child(&message_text);

    let _ = host.append_child(&callout);
}

fn remove_error_callout(host: &Element) {
    // We only ever inject a single callout, but query for the
    // sentinel attribute defensively in case more than one snuck in.
    let selector = format!("[{ERROR_CALLOUT_ATTR}]");
    let Ok(found) = host.query_selector_all(&selector) else {
        return;
    };
    for i in 0..found.length() {
        if let Some(node) = found.item(i)
            && let Some(el) = node.dyn_ref::<Element>()
        {
            el.remove();
        }
    }
}
