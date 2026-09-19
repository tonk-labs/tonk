//! Open account recovery over the missing-space screen.

use custom_elements::CustomElement;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, HtmlElement, window};

#[derive(Default)]
pub(crate) struct SpaceLogin {
    listener: Option<Closure<dyn FnMut(Event)>>,
}

impl CustomElement for SpaceLogin {
    fn shadow() -> bool {
        false
    }

    fn inject_children(&mut self, _this: &HtmlElement) {
        // Keep the native button authored by the profile view.
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let listener = Closure::wrap(Box::new(move |_event: Event| {
            // Omit `space`: a nonempty value asks the host to resume sharing.
            // The host owns the actual route; this is an opaque guest.
            tonk_host::request_registration(r#"{"reason":"space-login"}"#);
        }) as Box<dyn FnMut(Event)>);
        let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        self.listener = Some(listener);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(listener) = self.listener.take() {
            let _ = this
                .remove_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        }
    }
}

pub(crate) fn register() {
    let Some(elements) = window().map(|window| window.custom_elements()) else {
        return;
    };
    if elements.get("tonk-space-login").is_undefined() {
        SpaceLogin::define("tonk-space-login");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn recovery_opens_login_without_starting_a_share_and_reconnects_once() {
        register();
        let window = window().unwrap();
        let document = window.document().unwrap();
        let calls = Array::new();
        let recorded = calls.clone();
        let callback = Closure::<dyn FnMut(JsValue)>::new(move |payload| {
            recorded.push(&payload);
        });
        let bridge = Object::new();
        Reflect::set(
            &bridge,
            &"register".into(),
            callback.as_ref().unchecked_ref::<Function>(),
        )
        .unwrap();
        Reflect::set(&window, &"tonk".into(), &bridge).unwrap();
        let host = document.create_element("tonk-space-login").unwrap();
        host.set_inner_html(r#"<button type="button">sign in</button>"#);
        let button: HtmlElement = host.first_element_child().unwrap().unchecked_into();
        document.body().unwrap().append_child(&host).unwrap();
        button.click();
        host.remove();
        button.click();
        assert_eq!(calls.length(), 1, "a detached control has no listener");
        document.body().unwrap().append_child(&host).unwrap();
        button.click();
        assert_eq!(
            calls.length(),
            2,
            "reconnecting must not duplicate login requests"
        );
        assert_eq!(
            calls.get(1).as_string().unwrap(),
            r#"{"reason":"space-login"}"#
        );
        host.remove();
        Reflect::delete_property(window.unchecked_ref::<Object>(), &"tonk".into()).unwrap();
    }
}
