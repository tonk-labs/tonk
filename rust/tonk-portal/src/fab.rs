//! The `<tonk-fab-portal>` custom element.
//!
//! A [`TonkPortal`] variant that renders a sealed iframe as a small
//! fixed-position box positioned top-centre (default 64 × 64 px,
//! z-index near `MAX_SAFE_INTEGER - 1`). It shares all bridge and
//! lifecycle logic with `<tonk-portal>` through [`crate::shared`]; the
//! only difference is the iframe styling.
//!
//! Attributes: `content` (guest HTML string), `runtime` (boolean —
//! injects the guest element runtime before mounting `content`).
//!
//! Task 5 will add geometry handling (`__tonkFab` messages); this task
//! only delivers the correctly-positioned sealed box.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlIFrameElement, window};

use crate::bridge::{self, PortalState};
use crate::shared::{connect_portal, reload_portal};

/// The FAB portal custom element. Holds the shared [`PortalState`];
/// `None` until `connected_callback` builds it.
#[derive(Default)]
pub struct TonkFabPortal {
    inner: RefCell<Option<Rc<RefCell<PortalState>>>>,
}

impl CustomElement for TonkFabPortal {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["content", "runtime"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        connect_portal(this, &self.inner, |iframe| {
            // Fixed small box: top-centre, above all content. Task 5 will
            // expand / reposition via `__tonkFab` geometry messages.
            let style = iframe.style();
            let _ = style.set_property("position", "fixed");
            let _ = style.set_property("top", "12px");
            let _ = style.set_property("left", "50%");
            let _ = style.set_property("transform", "translateX(-50%)");
            let _ = style.set_property("width", "64px");
            let _ = style.set_property("height", "64px");
            let _ = style.set_property("border", "0");
            let _ = style.set_property("background", "transparent");
            let _ = style.set_property("z-index", "2147483646");
            let _ = style.set_property("color-scheme", "normal");
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(state) = self.inner.borrow_mut().take() {
            let mut s = state.borrow_mut();
            s.disposed = true;
            s.clear_subs();
            if let Some(iframe) = s.iframe.take() {
                bridge::unregister_portal(&iframe);
                if let Some(parent) = iframe.parent_node() {
                    let _ = parent.remove_child(&iframe);
                }
            }
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        let host: Element = this.clone().into();
        // Pre-connect callbacks (during upgrade) have no state yet; the
        // initial values are read live in `connected_callback`.
        let Some(state) = self.inner.borrow().clone() else {
            return;
        };
        if name == "content" {
            reload_portal(&host, &state);
        }
    }
}

/// Register `<tonk-fab-portal>` with the page. Idempotent. Installs the
/// page-level `hello` message listener (safe to call multiple times —
/// it is guarded by a thread-local), defines the element, and installs
/// the `reset` / `error` prototype shims.
pub fn register_fab_portal() {
    bridge::install_message_listener();
    if already_registered() {
        return;
    }
    TonkFabPortal::define("tonk-fab-portal");
    install_method_shims();
}

/// Install `reset` / `update` / `error` on the `<tonk-fab-portal>`
/// prototype, each forwarding to the per-instance `__tonk*` closure.
fn install_method_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-fab-portal");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let error_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkError === 'function') this.__tonkError(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
    let _ = Reflect::set(&proto, &"error".into(), &error_fn);
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-fab-portal").is_undefined()
}
