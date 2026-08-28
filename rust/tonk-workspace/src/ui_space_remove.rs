//! `<ui-space-remove>` — opens the shared modal around a seeded remove form.

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, Event, HtmlElement, window};

type EventClosure = Closure<dyn FnMut(Event)>;

#[derive(Default)]
struct UiSpaceRemove {
    click: Option<EventClosure>,
}

impl CustomElement for UiSpaceRemove {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        if self.click.is_some() {
            return;
        }
        let host = this.clone();
        let click: EventClosure = Closure::wrap(Box::new(move |event: Event| {
            let opens = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .and_then(|target| target.closest("[data-space-remove-open]").ok().flatten())
                .is_some();
            if !opens {
                return;
            }
            let Some(dialog) = host
                .query_selector("[data-space-remove-dialog]")
                .ok()
                .flatten()
            else {
                return;
            };
            let Some(show) = Reflect::get(dialog.as_ref(), &"show".into())
                .ok()
                .and_then(|show| show.dyn_into::<Function>().ok())
            else {
                return;
            };
            let _ = show.call0(dialog.as_ref());
        }));
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        self.click = Some(click);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
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

/// Register `<ui-space-remove>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else { return };
    if win.custom_elements().get("ui-space-remove").is_undefined() {
        UiSpaceRemove::define("ui-space-remove");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{Event, HtmlDialogElement, HtmlElement, window};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn it_opens_the_shared_dialog_and_restores_the_remove_button() {
        tonk_fab::register();
        super::register();
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("ui-space-remove")
            .expect("create remove element")
            .dyn_into()
            .expect("HtmlElement");
        host.set_inner_html(
            r#"<button type="button" data-space-remove-open>remove</button>
               <tonk-dialog data-space-remove-dialog heading="confirm space removal">
                 <form id="remove-test"></form>
                 <button slot="actions" type="button" data-dialog="close">cancel</button>
                 <button slot="actions" type="submit" form="remove-test" data-remove>remove space</button>
               </tonk-dialog>"#,
        );
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("append remove element");

        let opener: HtmlElement = host
            .query_selector("[data-space-remove-open]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        opener.focus().unwrap();
        opener.click();

        let dialog_host: HtmlElement = host
            .query_selector("tonk-dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let native: HtmlDialogElement = dialog_host
            .shadow_root()
            .expect("dialog shadow root")
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(native.open(), "the opener must call tonk-dialog.show()");

        host.query_selector("[data-dialog=close]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert!(!native.open(), "Cancel must close the native dialog");
        assert!(
            document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&opener))),
            "native dialog close must restore the remove button"
        );

        let submitted = Rc::new(Cell::new(false));
        let saw_submit = submitted.clone();
        let on_submit = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            event.prevent_default();
            saw_submit.set(true);
        });
        host.query_selector("form")
            .unwrap()
            .unwrap()
            .add_event_listener_with_callback("submit", on_submit.as_ref().unchecked_ref())
            .unwrap();
        opener.click();
        host.query_selector("[data-remove]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert!(
            submitted.get(),
            "the real remove action must submit its form"
        );
        host.query_selector("[data-dialog=close]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        host.remove();
    }
}
