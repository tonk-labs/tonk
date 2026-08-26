//! `<ui-mode-switch>` — the light/dark cap.
//!
//! Host chrome, not space content: a space cannot redefine it, and it themes
//! the CHROME around a view rather than the view itself.
//!
//! Mode follows the system until this is used; then the cap overrides it.
//! That ordering is the law — the cap OVERRIDES the system preference rather
//! than replacing it.
//!
//! The signal it writes is the app's existing one: `wa-dark` / `wa-light` on
//! the root element, the same classes the portal bridge stamps into every
//! guest from the page's own snapshot. Introducing a second theme signal for
//! the Hub alone would leave the two free to disagree.
//!
//! It goes through [`tonk_host::theme`] rather than setting the class here,
//! so the change also reaches guests nested below this one — the theme is one
//! property of the app, and each frame owns only its own root element.
//!
//! ## The choice does not survive a reload, and cannot yet
//!
//! Views render inside a sealed guest — `sandbox="allow-scripts"`, an opaque
//! origin — where `localStorage` throws and `document` is the guest's own,
//! not the page's. So there is nowhere here to put the preference: writing it
//! locally would be discarded, and the guest is re-seeded from the page's
//! class snapshot on every load.
//!
//! Persisting it needs one of two things that belong in their own change:
//! a `mode` page effect (`tonk-host::page_effect`, alongside `navigate` and
//! `title`) so the PAGE stores it and `index.html` can apply it before first
//! paint; or a profile claim through `window.tonk.transact`, the way the FAB
//! stores its dock — that route already works from inside a guest. Until
//! then this is deliberately session-scoped rather than pretending.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, window};

/// The class this element renders on its button, for the host view to style.
const CAP_CLASS: &str = "mode-cap";

/// A retained click-listener closure.
type ClickClosure = Closure<dyn FnMut(web_sys::Event)>;

/// Per-element state.
#[derive(Default)]
pub(crate) struct UiModeSwitch {
    click: Option<ClickClosure>,
}

impl CustomElement for UiModeSwitch {
    fn shadow() -> bool {
        // Light DOM: the cap is part of the chrome it sits in, and the view
        // that mounts it owns its geometry.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        // Reuse an existing control rather than appending a second one:
        // `inject_children` runs again whenever the element is re-created or
        // re-parented, and an unguarded append stacks duplicates. Mirrors
        // `ui_sync_status::paint`.
        if button_of(this).is_some() {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(button) = document.create_element("button") else {
            return;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("class", CAP_CLASS);
        let _ = button.set_attribute("role", "switch");
        let _ = button.set_attribute("title", "dark / light");
        let _ = button.set_attribute("aria-label", "dark mode");
        if let Ok(mark) = document.create_element("span") {
            mark.set_class_name("mode-mark");
            let _ = mark.set_attribute("aria-hidden", "true");
            let _ = button.append_child(&mark);
        }
        let _ = this.append_child(&button);
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Nothing to restore (see the module docs): the bridge has already
        // stamped the page's class onto this guest, so the cap only has to
        // report what is currently painted.
        reflect(this);

        if self.click.is_some() {
            return;
        }
        let host = this.clone();
        let click: ClickClosure = Closure::wrap(Box::new(move |_: web_sys::Event| {
            tonk_host::theme::set_mode(!tonk_host::theme::is_dark());
            reflect(&host);
        }));
        if let Some(button) = button_of(this) {
            let _ =
                button.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
        self.click = Some(click);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.click = None;
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

fn button_of(this: &HtmlElement) -> Option<Element> {
    this.query_selector(&format!(".{CAP_CLASS}")).ok().flatten()
}

/// Report the current state on the control.
fn reflect(this: &HtmlElement) {
    if let Some(button) = button_of(this) {
        let _ = button.set_attribute("aria-checked", &tonk_host::theme::is_dark().to_string());
    }
}

/// Register `<ui-mode-switch>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-mode-switch").is_undefined() {
        UiModeSwitch::define("ui-mode-switch");
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{HtmlElement, window};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn it_mounts_one_labelled_switch_with_one_split_tone_mark() {
        super::register();
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("ui-mode-switch")
            .expect("create mode switch")
            .dyn_into()
            .expect("HtmlElement");
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("append mode switch");

        let switches = host.query_selector_all("[role=\"switch\"]").unwrap();
        assert_eq!(switches.length(), 1, "the component owns one switch");
        let switch: web_sys::Element = switches
            .item(0)
            .unwrap()
            .dyn_into()
            .expect("switch element");
        assert_eq!(
            switch.get_attribute("aria-label").as_deref(),
            Some("dark mode"),
        );
        assert_eq!(
            host.query_selector_all(".mode-mark").unwrap().length(),
            1,
            "the square cell contains one split-tone mode mark",
        );
        host.remove();
    }
}
