//! `<ui-space-remove>` — names and routes delete/leave from one Hub row.

use custom_elements::CustomElement;
use js_sys::{Function, Reflect};
use tonk_worker_api::AccountStatus;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, window};

type EventClosure = Closure<dyn FnMut(Event)>;

const ACTION_DELETE_HOSTED: &str = "delete-hosted";
const ACTION_DELETE_LOCAL: &str = "delete-local";
const ACTION_LEAVE: &str = "leave";

fn value(this: &HtmlElement, name: &str) -> String {
    this.get_attribute(name)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn set_action(this: &HtmlElement, action: &str) {
    let _ = this.set_attribute("data-space-action", action);
    let label = if action == ACTION_LEAVE {
        "leave"
    } else {
        "delete"
    };
    if let Ok(Some(opener)) = this.query_selector("[data-space-remove-open]") {
        opener.set_text_content(Some(label));
        let _ = opener.remove_attribute("disabled");
    }
}

/// Decide the user-facing verb from durable directory facts. A provider names
/// the account that can delete the hosted space; comparing it with the local
/// account root keeps "delete" an authority statement rather than a guess.
fn classify_action(this: &HtmlElement) {
    let provider = value(this, "data-space-provider");
    if provider.is_empty() {
        set_action(
            this,
            if value(this, "data-space-founded").is_empty() {
                ACTION_LEAVE
            } else {
                ACTION_DELETE_LOCAL
            },
        );
        return;
    }

    let host = this.clone();
    spawn_local(async move {
        match tonk_host::get_json("/api/account").await {
            Ok(body) => match serde_json::from_str::<AccountStatus>(&body) {
                Ok(status) => {
                    let owner = match status {
                        AccountStatus::Registered { root_did, .. }
                        | AccountStatus::Unregistered { root_did, .. } => Some(root_did),
                        AccountStatus::RootMissing { .. } => None,
                    }
                    .is_some_and(|root| root == provider);
                    set_action(
                        &host,
                        if owner {
                            ACTION_DELETE_HOSTED
                        } else {
                            ACTION_LEAVE
                        },
                    );
                }
                Err(error) => {
                    tonk_common::log!("space action could not read account state: {error:?}");
                    if let Ok(Some(opener)) = host.query_selector("[data-space-remove-open]") {
                        opener.set_text_content(Some("unavailable"));
                    }
                }
            },
            Err(error) => {
                tonk_common::log!("space action could not read account ownership: {error:?}");
                if let Ok(Some(opener)) = host.query_selector("[data-space-remove-open]") {
                    opener.set_text_content(Some("unavailable"));
                }
            }
        }
    });
}

fn open_hosted_deletion(this: &HtmlElement) {
    let subject = value(this, "data-space-subject");
    if subject.is_empty() {
        return;
    }
    let Ok(params) = web_sys::UrlSearchParams::new() else {
        return;
    };
    params.append("delete-space", &subject);
    tonk_host::navigate_to(&format!("/settings?{}#delete-account", params.to_string()));
}

fn prepare_local_dialog(this: &HtmlElement, action: &str) {
    let name = value(this, "data-space-name");
    let name = if name.is_empty() { "this space" } else { &name };
    let (heading, copy, submit) = if action == ACTION_DELETE_LOCAL {
        (
            "confirm space deletion",
            format!(
                "Permanently delete {name} from this device? This space has no Tonk-hosted copy, so its local data will be gone for good."
            ),
            "delete space",
        )
    } else {
        (
            "confirm leaving space",
            format!(
                "Leave {name}? This removes the space and its local data from this device. You'll need another invite link to join again. Other members keep access."
            ),
            "leave space",
        )
    };
    if let Ok(Some(dialog)) = this.query_selector("[data-space-remove-dialog]") {
        let _ = dialog.set_attribute("heading", heading);
    }
    if let Ok(Some(form)) = this.query_selector("form[data-remove]") {
        form.set_text_content(Some(&copy));
    }
    if let Ok(Some(button)) = this.query_selector("[data-space-remove-submit]") {
        button.set_text_content(Some(submit));
    }
}

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
            let action = value(&host, "data-space-action");
            if action == ACTION_DELETE_HOSTED {
                open_hosted_deletion(&host);
                return;
            }
            if action != ACTION_DELETE_LOCAL && action != ACTION_LEAVE {
                return;
            }
            prepare_local_dialog(&host, &action);
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
        classify_action(this);
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

    fn action_host() -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("ui-space-remove")
            .expect("create remove element")
            .dyn_into()
            .expect("HtmlElement");
        host.set_inner_html(
            r#"<button type="button" data-space-remove-open disabled>checking</button>
               <tonk-dialog data-space-remove-dialog heading="confirm space removal">
                 <form id="remove-action" data-remove></form>
                 <button type="submit" form="remove-action" data-space-remove-submit></button>
               </tonk-dialog>"#,
        );
        host
    }

    #[wasm_bindgen_test]
    fn it_names_local_creation_deletion_and_joined_space_leaving() {
        let local = action_host();
        local.set_attribute("data-space-name", "notes").unwrap();
        local.set_attribute("data-space-founded", "1").unwrap();
        super::classify_action(&local);
        assert_eq!(
            local
                .query_selector("[data-space-remove-open]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("delete")
        );
        super::prepare_local_dialog(&local, super::ACTION_DELETE_LOCAL);
        assert!(
            local
                .query_selector("form")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("has no Tonk-hosted copy")
        );

        let joined = action_host();
        joined.set_attribute("data-space-name", "forum").unwrap();
        super::classify_action(&joined);
        assert_eq!(
            joined
                .query_selector("[data-space-remove-open]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("leave")
        );
        super::prepare_local_dialog(&joined, super::ACTION_LEAVE);
        assert!(
            joined
                .query_selector("form")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("need another invite link")
        );
    }

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
            r#"<button type="button" data-space-remove-open>leave</button>
               <tonk-dialog data-space-remove-dialog heading="confirm space removal">
                 <form id="remove-test"></form>
                 <button slot="actions" type="button" data-dialog="close">cancel</button>
                 <button slot="actions" type="submit" form="remove-test" data-space-remove-submit data-remove>leave space</button>
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
