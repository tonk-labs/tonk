//! `<tonk-title>` — a headless element that names the browser tab.
//!
//! It renders nothing. Its only job is to push its `text` attribute to
//! the host page, which owns `document.title`: a sealed guest cannot
//! touch the top document, so the text rides the bridge's `title`
//! message (`window.tonk.setTitle`) and the parent assigns it.
//!
//! DEPTH CONSTRAINT: the bridge dispatcher runs in the guest's PARENT
//! document, so this titles the real tab only when mounted in a
//! depth-1 guest — the profile chrome's space view. Mounted deeper it
//! silently retitles an intermediate iframe instead.

use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{HtmlElement, window};

#[derive(Default)]
struct TitleElement;

impl CustomElement for TitleElement {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["text", "hidden"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        push_title(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        push_title(this);
    }
}

/// Push the element's `text` to the host page over `window.tonk.setTitle`.
///
/// Best-effort at every step: a blank text (a view whose `{name}` has
/// not resolved) or an absent bridge (the element connected before the
/// bootstrap) leaves the tab exactly as it is.
///
/// A `<tonk-title>` mounted as a `<tonk-display>` slot fallback
/// (`slot="no-model"` etc.) is present in the DOM from the moment the
/// view template is cloned in — `tonk-display` only toggles `hidden` on
/// slot children, it never adds or removes them. So this element's own
/// `connected_callback` fires at mount regardless of which lifecycle
/// state is actually current, and would otherwise push a title before
/// the display has settled. `hidden` is the signal that this instance
/// is not the active slot; bail on it rather than push.
///
/// This is race-free because custom-element `connected_callback`s fire
/// parent-first in tree order: `<tonk-display>`'s `connected_callback`
/// calls `state::set(&host, State::Loading)` synchronously as its first
/// statement, which projects slots (sets `hidden` on every non-matching
/// slot child) before any child `<tonk-title>` connects. A later
/// transition to `no-model`/`no-entity` removes `hidden` from the
/// matching child, firing `attribute_changed_callback` and pushing the
/// title at the right moment. A non-slotted `<tonk-title>` (a plain
/// child of a view template, not a slot fallback) never receives
/// `hidden` and is unaffected.
fn push_title(this: &HtmlElement) {
    if this.has_attribute("hidden") {
        return;
    }
    let Some(text) = this.get_attribute("text").filter(|text| !text.is_empty()) else {
        return;
    };
    let Some(tonk) = window_tonk() else {
        return;
    };
    let Some(set_title) = get_fn(&tonk, "setTitle") else {
        return;
    };
    let _ = set_title.call1(&tonk, &JsValue::from_str(&text));
}

/// `window.tonk`, if the portal bootstrap installed it.
///
/// A deliberate local copy of the same helper in `tonk-guest`'s
/// `guest_host.rs`: `tonk-guest` depends on this crate, not the other
/// way round, so it cannot be imported. Twelve lines of boilerplate is
/// a smaller cost than hoisting a shared module through `tonk-host`
/// for a second caller. Hoist if a third appears.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|value| value.dyn_into::<Object>().ok())
}

/// A callable property off `window.tonk`.
fn get_fn(tonk: &Object, name: &str) -> Option<Function> {
    Reflect::get(tonk, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.dyn_into::<Function>().ok())
}

/// Register `<tonk-title>`. Call once from the guest's element surface.
pub fn register() {
    TitleElement::define("tonk-title");
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Install a `window.tonk.setTitle` stub recording its argument on
    /// `window.__title`, clearing any previous record. There is no real
    /// bridge in the test harness, so the stub stands in for the parent.
    fn install_stub() {
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("__title"), &JsValue::UNDEFINED);
        let tonk = Object::new();
        let capture = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
            let win = window().expect("a window in the test harness");
            let _ = Reflect::set(&win, &JsValue::from_str("__title"), &value);
        });
        let _ = Reflect::set(&tonk, &JsValue::from_str("setTitle"), capture.as_ref());
        capture.forget();
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &tonk);
    }

    fn captured() -> Option<String> {
        let win = window().expect("a window in the test harness");
        Reflect::get(&win, &JsValue::from_str("__title"))
            .ok()?
            .as_string()
    }

    fn element_with_text(text: Option<&str>) -> HtmlElement {
        let document = window()
            .expect("a window in the test harness")
            .document()
            .expect("a document in the test harness");
        let element = document
            .create_element("tonk-title")
            .expect("creates an element")
            .dyn_into::<HtmlElement>()
            .expect("an html element");
        if let Some(text) = text {
            let _ = element.set_attribute("text", text);
        }
        element
    }

    /// Only a non-empty `text` rides the bridge. A blank or absent one
    /// is dropped, so a view that has not resolved `{name}` yet never
    /// blanks the tab.
    #[dialog_common::test]
    async fn it_pushes_only_a_non_empty_text() {
        install_stub();
        push_title(&element_with_text(Some("Notes — Tonk")));
        assert_eq!(
            captured(),
            Some("Notes — Tonk".to_owned()),
            "a non-empty text should reach the bridge"
        );

        install_stub();
        push_title(&element_with_text(Some("")));
        assert_eq!(
            captured(),
            None,
            "an empty text should not reach the bridge"
        );

        install_stub();
        push_title(&element_with_text(None));
        assert_eq!(
            captured(),
            None,
            "a missing text attribute should not reach the bridge"
        );
    }

    /// A slot fallback stays in the DOM for the display's whole
    /// lifetime, `hidden` toggling which one is current. A hidden
    /// `<tonk-title>` must push nothing regardless of its `text`;
    /// clearing `hidden` must push it, mirroring the moment
    /// `tonk-display` projects the matching slot.
    #[dialog_common::test]
    async fn it_does_not_push_while_hidden() {
        install_stub();
        let element = element_with_text(Some("Untitled — Tonk"));
        let _ = element.set_attribute("hidden", "");

        push_title(&element);
        assert_eq!(
            captured(),
            None,
            "a hidden slot fallback should not push its text"
        );

        let _ = element.remove_attribute("hidden");
        push_title(&element);
        assert_eq!(
            captured(),
            Some("Untitled — Tonk".to_owned()),
            "clearing hidden should push the text"
        );
    }

    /// Without a bridge installed the push is a silent no-op, not a
    /// panic: the element may connect before the bootstrap does. A
    /// panic would fail this outright; the assertion additionally
    /// pins that nothing was pushed.
    #[dialog_common::test]
    async fn it_does_nothing_without_a_bridge() {
        install_stub();
        let win = window().expect("a window in the test harness");
        let _ = Reflect::set(&win, &JsValue::from_str("__title"), &JsValue::UNDEFINED);
        let _ = Reflect::set(&win, &JsValue::from_str("tonk"), &JsValue::UNDEFINED);

        push_title(&element_with_text(Some("Notes — Tonk")));

        assert_eq!(
            captured(),
            None,
            "an absent bridge should push nothing at all"
        );
    }
}
