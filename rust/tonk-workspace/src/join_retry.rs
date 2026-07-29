//! Retry control for transient join failures.

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{CustomEvent, CustomEventInit, Event, HtmlElement, window};

const RETRYABLE_KIND: &str = "unavailable";

#[derive(Default)]
pub(crate) struct TonkJoinRetry {
    listener: Option<Closure<dyn FnMut(Event)>>,
}

impl CustomElement for TonkJoinRetry {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["kind"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        render(this);
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        render(this);
        let host = this.clone();
        let listener = Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
            let init = CustomEventInit::new();
            init.set_bubbles(true);
            if let Ok(retry) = CustomEvent::new_with_event_init_dict("tonk:join-retry", &init) {
                let _ = host.dispatch_event(&retry);
            }
        }) as Box<dyn FnMut(Event)>);
        let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        self.listener = Some(listener);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        render(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(listener) = self.listener.take() {
            let _ = this
                .remove_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        }
    }
}

fn render(this: &HtmlElement) {
    if this.get_attribute("kind").as_deref() == Some(RETRYABLE_KIND) {
        this.set_inner_html(
            r#"<button type="button" class="join-error__retry">Try again</button>"#,
        );
    } else {
        this.set_inner_html("");
    }
}

pub(crate) fn register() {
    let Some(elements) = window().map(|window| window.custom_elements()) else {
        return;
    };
    if elements.get("tonk-join-retry").is_undefined() {
        TonkJoinRetry::define("tonk-join-retry");
    }
}
