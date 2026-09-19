//! `<tonk-editable>` — an inline-editable text control.
//!
//! A lightweight, input-like custom element for editing a single line of
//! text in place. It is `contenteditable`, shows its `value` attribute as
//! its text, exposes a `.value` property, and dispatches a `change` event
//! on commit — so a view can drive it exactly like a native input:
//!
//! ```html
//! <tonk-editable value={name} data-subject={this}
//!   onchange=tonk/rename-repository></tonk-editable>
//! ```
//!
//! The `change` event makes the element the `[data-onchange]` binding the
//! event delegation resolves, and `dom.event.current-target/value` reads
//! back the edited text. The keyboard convention a static template can't
//! express lives here: **Enter** commits (blur, which fires `change`) and
//! **Escape** cancels (restores the value the field had on focus, then
//! blurs, so no `change` with a new value is emitted).
//!
//! It holds no app policy — the commit rides on the consuming view's
//! `onchange` binding.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::cell::RefCell;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::rc::Rc;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use custom_elements::CustomElement;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::JsCast;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_sys::{Event, HtmlElement, KeyboardEvent, window};

/// Retained listeners, kept alive for the element's lifetime so the
/// closures stay valid while it is in the DOM.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
type Listeners = Rc<RefCell<Vec<Closure<dyn FnMut(Event)>>>>;

/// Per-element state: the listeners and the value captured on focus,
/// restored when the user presses Escape.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Default)]
pub(crate) struct TonkEditable {
    listeners: Listeners,
    focus_value: Rc<RefCell<String>>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl CustomElement for TonkEditable {
    fn shadow() -> bool {
        // Light DOM: the consuming view styles the text and the event
        // delegation must reach this element as the `[data-onchange]`
        // binding via `closest`.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // The bound value flows in through the `value` *property* (the
        // renderer assigns a string binding as a property when the name
        // exists on the element), not the attribute — so nothing to
        // observe here. The literal `value={name}` attribute on the
        // template clone is ignored; reading it would surface the
        // un-substituted `{name}`.
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Editing is opt-in on DOUBLE-CLICK, not a single click: the element
        // starts NON-editable (so a click just selects/does nothing) and only
        // turns `contenteditable` on when double-clicked, focusing to edit. It
        // reverts to non-editable on blur (see `on_blur`). This keeps a stray
        // single click from dropping the chip straight into edit mode.
        let _ = this.set_attribute("contenteditable", "false");
        let _ = this.set_attribute("role", "textbox");

        // Double-click enters edit mode: make it editable, then focus so the
        // caret lands and `on_focus` captures the pre-edit value.
        let on_dblclick = {
            let this = this.clone();
            Closure::wrap(Box::new(move |_event: Event| {
                let _ = this.set_attribute("contenteditable", "plaintext-only");
                let _ = this.focus();
            }) as Box<dyn FnMut(Event)>)
        };
        let _ =
            this.add_event_listener_with_callback("dblclick", on_dblclick.as_ref().unchecked_ref());

        // Capture the value on focus so Escape can restore it.
        let focus_value = self.focus_value.clone();
        let on_focus = {
            let this = this.clone();
            Closure::wrap(Box::new(move |_event: Event| {
                *focus_value.borrow_mut() = this.text_content().unwrap_or_default();
            }) as Box<dyn FnMut(Event)>)
        };
        let _ = this.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());

        // Enter commits (blur → `change`); Escape restores then blurs.
        let focus_value = self.focus_value.clone();
        let on_keydown = {
            let this = this.clone();
            Closure::wrap(Box::new(move |event: Event| {
                let Some(event) = event.dyn_ref::<KeyboardEvent>() else {
                    return;
                };
                match event.key().as_str() {
                    "Enter" => {
                        event.prevent_default();
                        let _ = this.blur();
                    }
                    "Escape" => {
                        event.prevent_default();
                        this.set_text_content(Some(&focus_value.borrow()));
                        let _ = this.blur();
                    }
                    _ => {}
                }
            }) as Box<dyn FnMut(Event)>)
        };
        let _ =
            this.add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref());

        // On blur, emit a `change` event so the consuming view's
        // `onchange` binding fires. It must **bubble**: `tonk-display`
        // delegates events on an ancestor (`<tonk-view>`) and matches the
        // closest `[data-onchange]` as it bubbles up, so a non-bubbling
        // event (the `Event::new` default) would never reach the
        // listener and the command would silently never fire. The
        // binding reads the edited text via
        // `dom.event.current-target/value`, our prototype `value` getter.
        let focus_value = self.focus_value.clone();
        let on_blur = {
            let this = this.clone();
            Closure::wrap(Box::new(move |_event: Event| {
                let current = this.text_content().unwrap_or_default();
                // Leaving edit mode: revert to non-editable so the next single
                // click doesn't re-enter it (double-click is required again).
                let _ = this.set_attribute("contenteditable", "false");
                // No-op when nothing changed (e.g. Escape just restored
                // the original) — avoids a redundant rename round-trip.
                if current == *focus_value.borrow() {
                    return;
                }
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                if let Ok(event) = Event::new_with_event_init_dict("change", &init) {
                    let _ = this.dispatch_event(&event);
                }
            }) as Box<dyn FnMut(Event)>)
        };
        let _ = this.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref());

        self.listeners
            .borrow_mut()
            .extend([on_dblclick, on_focus, on_keydown, on_blur]);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.listeners.borrow_mut().clear();
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Register `<tonk-editable>` and install a `value` getter on its
/// prototype so `dom.event.current-target/value` reads the edited text.
/// Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn register() {
    let Some(elements) = window().map(|w| w.custom_elements()) else {
        return;
    };
    if !elements.get("tonk-editable").is_undefined() {
        return;
    }
    TonkEditable::define("tonk-editable");
    install_value_accessor();
}

/// Define a `value` accessor on the `<tonk-editable>` prototype, with a
/// getter returning the element's text and a setter writing it (so the
/// element behaves like a native input's `value`). Both halves matter:
/// the renderer applies a string binding as a *property* when the name
/// exists on the element (`tonk-display`'s `apply_attribute_binding`),
/// so without the setter `value={name}` would silently no-op and the
/// element would keep the literal `{name}` it cloned. The getter lets
/// `dom.event.current-target/value` read the edited text on commit.
/// The setter skips while focused so a remote update never clobbers an
/// in-progress edit.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn install_value_accessor() {
    use js_sys::{Function, Object, Reflect};

    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("tonk-editable");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let getter = Function::new_no_args("return this.textContent ?? '';");
    let setter = Function::new_with_args(
        "v",
        "if (this !== this.ownerDocument?.activeElement) this.textContent = v ?? '';",
    );
    let descriptor = Object::new();
    let _ = Reflect::set(&descriptor, &"get".into(), &getter);
    let _ = Reflect::set(&descriptor, &"set".into(), &setter);
    let _ = Reflect::set(&descriptor, &"configurable".into(), &JsValue::TRUE);
    let _ = Object::define_property(&Object::from(proto), &"value".into(), &descriptor);
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::KeyboardEventInit;

    wasm_bindgen_test_configure!(run_in_browser);

    fn mounted() -> HtmlElement {
        register();
        let document = window().unwrap().document().unwrap();
        let host: HtmlElement = document
            .create_element("tonk-editable")
            .unwrap()
            .dyn_into()
            .unwrap();
        host.set_text_content(Some("before"));
        document.body().unwrap().append_child(&host).unwrap();
        host
    }

    /// Begin an edit the way a person does. A single click deliberately
    /// does nothing — `contenteditable` starts `false` so a stray click
    /// cannot drop the chip into edit mode — so the gesture is a
    /// double-click, and focusing without one tests nothing.
    fn begin_edit(host: &HtmlElement) {
        let event = web_sys::MouseEvent::new("dblclick").unwrap();
        host.dispatch_event(&event).unwrap();
    }

    fn press(host: &HtmlElement, key: &str) {
        let init = KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        host.dispatch_event(&event).unwrap();
    }

    /// Enter ends the edit.
    ///
    /// A contenteditable does not commit on Enter by itself — it inserts
    /// a newline. Blurring is what makes the browser emit the `change`
    /// the consuming view's binding listens for, so a field that stays
    /// focused is one whose edit is never saved.
    ///
    /// This lived on `<ui-account-settings>` while that element owned
    /// the display-name input. The input is a view now, and the commit
    /// gesture belongs to whichever element provides it.
    #[wasm_bindgen_test]
    fn it_ends_the_edit_on_enter() {
        let host = mounted();
        begin_edit(&host);
        let document = window().unwrap().document().unwrap();
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&host))),
            "the edit must begin focused",
        );

        press(&host, "Enter");

        assert!(
            !document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&host))),
            "Enter must blur the field so the change-save path runs",
        );
        host.remove();
    }

    /// Escape restores what was there and ends the edit, so an abandoned
    /// edit cannot commit the half-typed value on the way out.
    #[wasm_bindgen_test]
    fn it_restores_the_previous_value_on_escape() {
        let host = mounted();
        begin_edit(&host);
        host.set_text_content(Some("half-typed"));

        press(&host, "Escape");

        assert_eq!(
            host.text_content().as_deref(),
            Some("before"),
            "Escape must put back what the edit started from",
        );
        host.remove();
    }
}
