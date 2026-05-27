//! `<tonk-branch name="…">` — annotates `detail.branch` on
//! outbound consumer events as they bubble.
//!
//! Same annotator pattern as `<tonk-repository>` but writes the
//! `branch` field instead of `space`.

use std::cell::RefCell;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use web_sys::{CustomEvent, Element, HtmlElement, window};

use crate::repository::{NamedListener, attach_annotators};

/// Outer per-element struct.
#[derive(Default)]
pub(crate) struct TonkBranch {
    listeners: RefCell<Vec<NamedListener>>,
}

impl CustomElement for TonkBranch {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["name"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let host: Element = this.clone().into();
        let installed = attach_annotators(&host, "branch");
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

fn dispatch_context_refresh(this: &HtmlElement) {
    let Some(_win) = window() else { return };
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    let Ok(ev) = CustomEvent::new_with_event_init_dict(crate::events::CONTEXT_REFRESH, &init)
    else {
        return;
    };
    let target: &Element = this.unchecked_ref();
    let _ = target.dispatch_event(&ev);
}

/// Register `<tonk-branch>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkBranch::define("tonk-branch");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-branch").is_undefined()
}
