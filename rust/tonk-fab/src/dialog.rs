//! `<tonk-dialog>` — a cluster.
//!
//! Glass blocks in the bar's own material, over a native `<dialog>` so the
//! platform owns the modal semantics (focus trap, inert background, Escape).
//! The `::backdrop` dims the page; the modal surface never dims — at panel
//! density (.92 light / .88 dark) the blur reads as nothing, so the filter
//! retires and only the glass colour stays.
//!
//! Shape follows the cap law: the header is boxy on the left and takes the
//! 18px cap only when a `side` rail is slotted, and the × sticks out to the
//! right of the boxy zone. The footer run renders only when actions are
//! slotted, and those actions fuse flush — the fill boundary is the divider.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Object;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlDialogElement, HtmlElement, window};

use crate::shadow::{self, Bound};

const CSS: &str = r#"
dialog{ background:transparent; border:0; padding:0; max-width:26rem; width:calc(100% - 40px); overflow:visible; }
:host([wide]) dialog{ max-width:36rem; }
dialog::backdrop{ background:rgba(16,16,12,.32); }
.stack{ display:flex; flex-direction:column; gap:7px; }
.hrow{ display:flex; gap:7px; }
.blk{ background:var(--_panel); box-shadow:var(--_ring); }
.t{ flex:1; height:36px; display:flex; align-items:flex-end; justify-content:flex-end;
  padding:0 12px 9px 16px; margin:0;
  font-size:13px; font-weight:600; text-transform:lowercase; color:var(--_ink); }
.w.has-side .t{ border-radius:18px 0 0 18px; }
.x{ width:36px; height:36px; flex:none; display:grid; place-items:center; border-radius:0 18px 18px 0;
  font-size:15px; color:var(--_ink); }
.x:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_panel); }
.x:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_panel); }
.main{ display:flex; gap:7px; align-items:stretch; margin-right:43px; }
.body{ flex:1; min-width:0; padding:14px 18px;
  font-size:13.5px; font-weight:400; line-height:1.55; color:var(--_soft);
  max-height:min(60vh, 420px); overflow:auto; }
.frow{ display:none; gap:0; justify-content:flex-end; margin-right:43px; }
.w.has-acts .frow{ display:flex; }
"#;

const HTML: &str = r#"<dialog>
  <div class="w">
    <div class="stack">
      <div class="hrow">
        <h3 class="t blk" part="heading"></h3>
        <button class="x blk" aria-label="close">&#215;</button>
      </div>
      <div class="main">
        <slot name="side"></slot>
        <div class="body blk" part="body"><slot></slot></div>
      </div>
      <div class="frow"><slot name="actions"></slot></div>
    </div>
  </div>
</dialog>"#;

/// Per-element state.
#[derive(Default)]
pub(crate) struct TonkDialog {
    listeners: Vec<Bound>,
    wired: Rc<RefCell<bool>>,
}

impl CustomElement for TonkDialog {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["mode", "heading"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        if *self.wired.borrow() {
            return;
        }
        *self.wired.borrow_mut() = true;

        let root = shadow::build(this, CSS, HTML);
        install_open_close_api(this);

        if let Ok(Some(close)) = root.query_selector(".x") {
            let host = this.clone();
            self.listeners
                .push(shadow::on_click(&close, move || close_dialog(&host)));
        }

        if let Ok(Some(dialog)) = root.query_selector("dialog") {
            // Escape reaches the native dialog as `cancel`; report it as a
            // close so a caller listening for `fabb-close` hears every exit.
            let host = this.clone();
            self.listeners
                .push(shadow::bind(&dialog, "cancel", move |_| {
                    shadow::emit(&host, "fabb-close", &JsValue::NULL);
                }));

            // A click landing on the dialog element itself is a click on the
            // backdrop — the content sits in `.w` inside it.
            let host = this.clone();
            let dialog_target = dialog.clone();
            self.listeners
                .push(shadow::bind(&dialog, "click", move |ev| {
                    if ev
                        .target()
                        .and_then(|t| t.dyn_into::<Element>().ok())
                        .is_some_and(|t| t == dialog_target)
                    {
                        close_dialog(&host);
                    }
                }));
        }

        // `data-dialog="close"` on any slotted control dismisses the cluster,
        // so a "not now" action needs no per-prompt wiring. Listened for on
        // the host, since the control is light-DOM content.
        {
            let host = this.clone();
            self.listeners.push(shadow::bind(this, "click", move |ev| {
                let dismisses = ev
                    .target()
                    .and_then(|t| t.dyn_into::<Element>().ok())
                    .and_then(|t| t.closest("[data-dialog=close]").ok().flatten())
                    .is_some();
                if dismisses {
                    close_dialog(&host);
                }
            }));
        }

        for slot in ["slot[name=side]", "slot[name=actions]"] {
            if let Ok(Some(element)) = root.query_selector(slot) {
                let host = this.clone();
                self.listeners
                    .push(shadow::bind(&element, "slotchange", move |_| {
                        sync_slots(&host)
                    }));
            }
        }

        self.listeners.push(shadow::install_visibility_pause(this));
        if let Some(listener) = shadow::install_system_mode(this) {
            self.listeners.push(listener);
        }
        sync_slots(this);
        sync_heading(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.listeners.clear();
        *self.wired.borrow_mut() = false;
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        match name.as_str() {
            "mode" => {
                shadow::apply_mode(this);
                propagate(this);
            }
            "heading" => sync_heading(this),
            _ => {}
        }
    }
}

/// Expose `show()` / `close()` and an `open` property, so callers drive the
/// cluster the same way they drive a native `<dialog>`.
///
/// `custom-elements` gives no route to add prototype members, so these are
/// installed as own properties on each instance. `open` is a real accessor
/// rather than a plain field: callers set `dialog.open = true` to raise a
/// prompt, and reading it back has to report whether the dialog is actually
/// showing, not what was last assigned.
fn install_open_close_api(this: &HtmlElement) {
    let host = this.clone();
    let show = Closure::<dyn FnMut()>::new(move || show_dialog(&host));
    let _ = js_sys::Reflect::set(this, &"show".into(), show.as_ref());
    show.forget();

    let host = this.clone();
    let close = Closure::<dyn FnMut()>::new(move || close_dialog(&host));
    let _ = js_sys::Reflect::set(this, &"close".into(), close.as_ref());
    close.forget();

    let descriptor = Object::new();
    let host = this.clone();
    let getter = Closure::<dyn FnMut() -> bool>::new(move || {
        native_dialog(&host).is_some_and(|dialog| dialog.open())
    });
    let _ = js_sys::Reflect::set(&descriptor, &"get".into(), getter.as_ref());
    getter.forget();

    let host = this.clone();
    let setter = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
        if value.is_truthy() {
            show_dialog(&host);
        } else {
            close_dialog(&host);
        }
    });
    let _ = js_sys::Reflect::set(&descriptor, &"set".into(), setter.as_ref());
    setter.forget();
    let _ = js_sys::Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE);
    let _ = Object::define_property(this, &"open".into(), &descriptor);
}

fn native_dialog(this: &HtmlElement) -> Option<HtmlDialogElement> {
    this.shadow_root()?
        .query_selector("dialog")
        .ok()
        .flatten()
        .and_then(|d| d.dyn_into::<HtmlDialogElement>().ok())
}

/// Open the cluster modally.
pub(crate) fn show_dialog(this: &HtmlElement) {
    shadow::apply_mode(this);
    propagate(this);
    if let Some(dialog) = native_dialog(this) {
        let _ = dialog.show_modal();
        shadow::emit(this, "fabb-open", &JsValue::NULL);
    }
}

/// Close the cluster.
pub(crate) fn close_dialog(this: &HtmlElement) {
    if let Some(dialog) = native_dialog(this) {
        dialog.close();
    }
    shadow::emit(this, "fabb-close", &JsValue::NULL);
}

/// The header takes its left cap only with a rail; the footer run appears
/// only with actions. Both are read from the light tree rather than the
/// slots, for the same reason as `mi::sync_sub`.
fn sync_slots(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };
    let Ok(Some(wrapper)) = root.query_selector(".w") else {
        return;
    };
    let has_side = matches!(this.query_selector("[slot=side]"), Ok(Some(_)));
    let has_acts = matches!(this.query_selector("[slot=actions]"), Ok(Some(_)));
    let _ = wrapper.class_list().toggle_with_force("has-side", has_side);
    let _ = wrapper.class_list().toggle_with_force("has-acts", has_acts);
}

fn sync_heading(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };
    if let Ok(Some(heading)) = root.query_selector(".t") {
        heading.set_text_content(Some(&this.get_attribute("heading").unwrap_or_default()));
    }
}

/// Hand the resolved mode to the FABB children the cluster hosts.
fn propagate(this: &HtmlElement) {
    let Ok(children) = this.query_selector_all("tonk-button,tonk-toggle,tonk-menu,tonk-field")
    else {
        return;
    };
    for index in 0..children.length() {
        let Some(node) = children.item(index) else {
            continue;
        };
        if let Ok(element) = node.dyn_into::<Element>() {
            shadow::pass_mode(this, &element);
        }
    }
}

/// Register `<tonk-dialog>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else { return };
    if win.custom_elements().get("tonk-dialog").is_undefined() {
        TonkDialog::define("tonk-dialog");
    }
}
