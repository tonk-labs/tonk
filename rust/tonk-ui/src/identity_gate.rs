//! Top-document gate for durable operations that require a local root.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use tonk_worker_api::{IdentityIntent, IdentityRequired, JoinResponse, RootStatus};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlButtonElement, HtmlElement, MessageEvent};

use crate::identity_bridge::{CreateRootInput, RootOutput, create_root, evaluate_root};

const STYLE_ID: &str = "tonk-identity-gate-styles";

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static GATE_STATE: RefCell<GateState> = const { RefCell::new(GateState {
        active: false,
        pending: VecDeque::new(),
    }) };
    static PREVIOUS_FOCUS: RefCell<Option<Element>> = const { RefCell::new(None) };
}

struct GateState {
    active: bool,
    pending: VecDeque<IdentityIntent>,
}

#[derive(Clone, Copy)]
enum RootMethod {
    Create,
    Evaluate,
}

fn ensure_stylesheet() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    style.set_id(STYLE_ID);
    style.set_text_content(Some(include_str!("identity_gate.css")));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
    }
}

fn isolate_gate(modal: &Element, primary: &HtmlButtonElement) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    PREVIOUS_FOCUS.with(|focus| *focus.borrow_mut() = document.active_element());
    if let Some(body) = document.body() {
        let children = body.children();
        for index in 0..children.length() {
            let Some(child) = children.item(index) else {
                continue;
            };
            if child.is_same_node(Some(modal)) || child.has_attribute("inert") {
                continue;
            }
            let _ = child.set_attribute("inert", "");
            let _ = child.set_attribute("data-tonk-gate-inert", "");
        }
    }
    let _ = primary.focus();
}

fn restore_document() {
    if let Some(document) = web_sys::window().and_then(|window| window.document())
        && let Some(body) = document.body()
    {
        let children = body.children();
        for index in 0..children.length() {
            let Some(child) = children.item(index) else {
                continue;
            };
            if child.has_attribute("data-tonk-gate-inert") {
                let _ = child.remove_attribute("inert");
                let _ = child.remove_attribute("data-tonk-gate-inert");
            }
        }
    }
    PREVIOUS_FOCUS.with(|focus| {
        if let Some(element) = focus.borrow_mut().take()
            && let Some(element) = element.dyn_ref::<HtmlElement>()
        {
            let _ = element.focus();
        }
    });
}

async fn request_root(method: RootMethod, device_did: String) -> Result<RootOutput, String> {
    // The gate provisions a root to finish an interrupted intent — creating a
    // spot, joining durably. No address is in hand, so the credential goes
    // unlabelled rather than mislabelled.
    let input = CreateRootInput {
        device_did,
        label: None,
    };
    match method {
        RootMethod::Create => create_root(input).await,
        RootMethod::Evaluate => evaluate_root(input).await,
    }
    .map_err(|error| error.to_string())
}

async fn replay(intent: IdentityIntent) -> Result<(), String> {
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| "window origin is unavailable".to_string())?;
    let client = reqwest::Client::new();
    match intent {
        IdentityIntent::CreateSpace {
            name,
            remote,
            revocation_url,
            template,
        } => {
            let response = client
                .post(format!("{origin}/api/spaces"))
                .json(&tonk_worker_api::CreateSpaceRequest {
                    name,
                    remote,
                    revocation_url,
                    template,
                })
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!("operation failed with {}", response.status()));
            }
            let created: tonk_worker_api::CreateSpaceResponse =
                response.json().await.map_err(|error| error.to_string())?;
            tonk_host::navigate_to(&format!("/space/{}", created.key));
            Ok(())
        }
        IdentityIntent::DurableJoin { url } => {
            let response = client
                .post(format!("{origin}/api/profile/join"))
                .json(&tonk_worker_api::JoinRequest { url })
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!("operation failed with {}", response.status()));
            }
            let joined: JoinResponse = response.json().await.map_err(|error| error.to_string())?;
            let repository = match joined {
                JoinResponse::Joined { repository } | JoinResponse::Renewed { repository } => {
                    repository
                }
            };
            tonk_host::navigate_to(&format!("/space/{}", repository.name));
            Ok(())
        }
    }
}

fn finish(modal: &Element) {
    modal.remove();
    restore_document();
    release_gate();
    pump();
}

fn queue_intent(intent: IdentityIntent) {
    GATE_STATE.with(|state| state.borrow_mut().pending.push_back(intent));
}

fn take_next_intent() -> Option<IdentityIntent> {
    GATE_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.active {
            return None;
        }
        let intent = state.pending.pop_front()?;
        state.active = true;
        Some(intent)
    })
}

fn release_gate() {
    GATE_STATE.with(|state| state.borrow_mut().active = false);
}

fn enqueue(intent: IdentityIntent) {
    queue_intent(intent);
    pump();
}

fn pump() {
    let Some(intent) = take_next_intent() else {
        return;
    };
    spawn_local(async move {
        if let Err(error) = show(intent).await {
            release_gate();
            if let Some(window) = web_sys::window() {
                let _ = window.alert_with_message(&error);
            }
            pump();
        }
    });
}

fn set_status(modal: &Element, message: &str) {
    if let Ok(Some(status)) = modal.query_selector("#tonk-identity-status") {
        status.set_text_content(Some(message));
    }
}

fn begin_action(modal: &Element) -> bool {
    if modal.has_attribute("data-running") {
        return false;
    }
    modal.set_attribute("data-running", "").is_ok()
}

fn bind_link_action(
    button: HtmlButtonElement,
    method: RootMethod,
    device_did: String,
    modal: Element,
) {
    let closure = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        if !begin_action(&modal) {
            return;
        }
        let modal = modal.clone();
        let device_did = device_did.clone();
        set_status(&modal, "Waiting for your passkey…");
        spawn_local(async move {
            match request_root(method, device_did).await {
                Ok(output) => {
                    let response = serde_json::json!({
                        "credentialId": output.credential_id,
                        "delegationHex": output.delegation_hex,
                    });
                    if let Ok(Some(element)) = modal.query_selector("#tonk-link-response") {
                        element.set_text_content(Some(&response.to_string()));
                        let _ = element.remove_attribute("hidden");
                    }
                    set_status(&modal, "Copy this response back to the terminal.");
                }
                Err(error) => set_status(&modal, &error),
            }
            let _ = modal.remove_attribute("data-running");
        });
    });
    let _ = button.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn show_cli_link() -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "window is unavailable".to_string())?;
    let fragment = window.location().hash().map_err(js_string)?;
    let fields: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(fragment.trim_start_matches('#').as_bytes())
            .into_owned()
            .collect();
    let device_did = fields
        .get("deviceDid")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| "the identity link is missing its device DID".to_string())?;
    let challenge = fields.get("challenge").cloned().unwrap_or_default();
    let document = window
        .document()
        .ok_or_else(|| "document is unavailable".to_string())?;
    let modal = document.create_element("section").map_err(js_string)?;
    modal.set_id("tonk-identity-link");
    modal.set_attribute("role", "dialog").map_err(js_string)?;
    modal.set_inner_html(
        r#"<div class="tonk-identity-card">
<h2>Link this command-line device</h2>
<p>The delegation is cryptographically bound to <code id="tonk-link-device"></code>.</p>
<p>Terminal challenge: <code id="tonk-link-challenge"></code></p>
<div class="tonk-identity-actions">
<button id="tonk-link-create" type="button">Create a new passkey</button>
<button id="tonk-link-existing" type="button">Use an existing passkey</button>
</div>
<p id="tonk-identity-status" role="status" aria-live="polite"></p>
<pre id="tonk-link-response" hidden></pre>
</div>"#,
    );
    if let Ok(Some(element)) = modal.query_selector("#tonk-link-device") {
        element.set_text_content(Some(&device_did));
    }
    if let Ok(Some(element)) = modal.query_selector("#tonk-link-challenge") {
        element.set_text_content(Some(&challenge));
    }
    document
        .body()
        .ok_or_else(|| "document body is unavailable".to_string())?
        .append_child(&modal)
        .map_err(js_string)?;
    let create: HtmlButtonElement = modal
        .query_selector("#tonk-link-create")
        .map_err(js_string)?
        .ok_or_else(|| "create button is missing".to_string())?
        .dyn_into()
        .map_err(|_| "create button is invalid".to_string())?;
    let existing: HtmlButtonElement = modal
        .query_selector("#tonk-link-existing")
        .map_err(js_string)?
        .ok_or_else(|| "existing button is missing".to_string())?
        .dyn_into()
        .map_err(|_| "existing button is invalid".to_string())?;
    isolate_gate(&modal, &create);
    bind_link_action(
        create,
        RootMethod::Create,
        device_did.clone(),
        modal.clone(),
    );
    bind_link_action(existing, RootMethod::Evaluate, device_did, modal);
    Ok(())
}

fn bind_action(
    button: HtmlButtonElement,
    method: RootMethod,
    device_did: String,
    intent: IdentityIntent,
    modal: Element,
) {
    let closure = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        if !begin_action(&modal) {
            return;
        }
        let modal = modal.clone();
        let device_did = device_did.clone();
        let intent = intent.clone();
        set_status(&modal, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let output = request_root(method, device_did).await?;
                if !modal.is_connected() {
                    return Ok(());
                }
                crate::api::save_root(output.credential_id, output.delegation_hex)
                    .await
                    .map_err(|error| error.to_string())?;
                if !modal.is_connected() {
                    return Ok(());
                }
                set_status(&modal, "Continuing…");
                replay(intent).await
            }
            .await;
            match result {
                Ok(()) if modal.is_connected() => finish(&modal),
                Ok(()) => {}
                Err(error) => {
                    let _ = modal.remove_attribute("data-running");
                    set_status(&modal, &error);
                }
            }
        });
    });
    let _ = button.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

async fn show(intent: IdentityIntent) -> Result<(), String> {
    let status = crate::api::root_status()
        .await
        .map_err(|error| error.to_string())?;
    let device_did = match status {
        RootStatus::Missing { device_did } | RootStatus::Ready { device_did, .. } => device_did,
    };
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "document is unavailable".to_string())?;
    let modal = document.create_element("section").map_err(js_string)?;
    modal.set_id("tonk-identity-gate");
    modal.set_attribute("role", "dialog").map_err(js_string)?;
    modal
        .set_attribute("aria-modal", "true")
        .map_err(js_string)?;
    modal
        .set_attribute("aria-labelledby", "tonk-identity-title")
        .map_err(js_string)?;
    modal
        .set_attribute(
            "aria-describedby",
            "tonk-identity-description tonk-identity-status",
        )
        .map_err(js_string)?;
    modal.set_inner_html(
        r#"<div class="tonk-identity-card">
<h2 id="tonk-identity-title">Create your local identity</h2>
<p id="tonk-identity-description">This durable action needs a passkey root stored on this device.</p>
<div class="tonk-identity-actions">
<button id="tonk-create-root" type="button">Create a new passkey</button>
<button id="tonk-use-root" type="button">Use an existing passkey</button>
<button id="tonk-cancel-root" type="button">Cancel</button>
</div>
<p id="tonk-identity-status" role="status" aria-live="polite"></p>
</div>"#,
    );
    let body = document
        .body()
        .ok_or_else(|| "document body is unavailable".to_string())?;
    body.append_child(&modal).map_err(js_string)?;
    let create: HtmlButtonElement = modal
        .query_selector("#tonk-create-root")
        .map_err(js_string)?
        .ok_or_else(|| "create button is missing".to_string())?
        .dyn_into()
        .map_err(|_| "create button is invalid".to_string())?;
    let existing: HtmlButtonElement = modal
        .query_selector("#tonk-use-root")
        .map_err(js_string)?
        .ok_or_else(|| "existing button is missing".to_string())?
        .dyn_into()
        .map_err(|_| "existing button is invalid".to_string())?;
    let cancel: HtmlButtonElement = modal
        .query_selector("#tonk-cancel-root")
        .map_err(js_string)?
        .ok_or_else(|| "cancel button is missing".to_string())?
        .dyn_into()
        .map_err(|_| "cancel button is invalid".to_string())?;
    let cancel_modal = modal.clone();
    let cancel_action = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        finish(&cancel_modal);
    });
    let _ =
        cancel.add_event_listener_with_callback("click", cancel_action.as_ref().unchecked_ref());
    cancel_action.forget();
    isolate_gate(&modal, &create);
    bind_action(
        create,
        RootMethod::Create,
        device_did.clone(),
        intent.clone(),
        modal.clone(),
    );
    bind_action(existing, RootMethod::Evaluate, device_did, intent, modal);
    Ok(())
}

fn js_string(error: JsValue) -> String {
    format!("{error:?}")
}

/// Install the top-document service-worker identity-request listener.
pub fn install() {
    if INSTALLED.with(|installed| installed.replace(true)) {
        return;
    }
    ensure_stylesheet();
    let Some(window) = web_sys::window() else {
        return;
    };
    if window.location().pathname().ok().as_deref() == Some("/identity/link") {
        if let Err(error) = show_cli_link() {
            let _ = window.alert_with_message(&error);
        }
        return;
    }
    let service_worker = window.navigator().service_worker();
    let listener = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Ok(message) = serde_wasm_bindgen::from_value::<IdentityRequired>(event.data()) else {
            return;
        };
        if message.message_type != "identity-required" {
            return;
        }
        enqueue(message.intent);
    });
    let _ = service_worker
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    listener.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_processes_identity_intents_in_fifo_order() {
        GATE_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.active = false;
            state.pending.clear();
        });
        queue_intent(IdentityIntent::CreateSpace {
            name: "first".into(),
            remote: None,
            revocation_url: None,
            template: None,
        });
        queue_intent(IdentityIntent::DurableJoin {
            url: "https://example.test/join#second".into(),
        });

        let Some(IdentityIntent::CreateSpace { name, .. }) = take_next_intent() else {
            panic!("first queued intent should create a space");
        };
        assert_eq!(name, "first");
        assert!(
            take_next_intent().is_none(),
            "only one prompt may be active"
        );
        release_gate();
        let Some(IdentityIntent::DurableJoin { url }) = take_next_intent() else {
            panic!("second queued intent should be the join");
        };
        assert!(url.ends_with("#second"));
        release_gate();
    }

    #[dialog_common::test]
    async fn it_isolates_focus_and_restores_the_document_on_cancel() {
        let document = web_sys::window().unwrap().document().unwrap();
        let sibling = document.create_element("button").unwrap();
        sibling.set_id("tonk-gate-focus-before");
        document.body().unwrap().append_child(&sibling).unwrap();
        sibling.dyn_ref::<HtmlElement>().unwrap().focus().unwrap();

        let modal = document.create_element("section").unwrap();
        modal.set_inner_html(
            r#"<button id="tonk-create-root">Create a new passkey</button>
<button id="tonk-use-root">Use an existing passkey</button>"#,
        );
        document.body().unwrap().append_child(&modal).unwrap();
        let primary: HtmlButtonElement = modal
            .query_selector("#tonk-create-root")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();

        isolate_gate(&modal, &primary);
        assert!(sibling.has_attribute("inert"));
        assert!(
            document
                .active_element()
                .is_some_and(|element| element.is_same_node(Some(primary.as_ref()))),
            "the primary action receives focus"
        );

        GATE_STATE.with(|state| state.borrow_mut().active = true);
        finish(&modal);
        assert!(!modal.is_connected());
        assert!(!sibling.has_attribute("inert"));
        assert!(
            document
                .active_element()
                .is_some_and(|element| element.is_same_node(Some(&sibling))),
            "focus returns to the previously active control"
        );
        GATE_STATE.with(|state| assert!(!state.borrow().active));
        sibling.remove();
    }

    #[dialog_common::test]
    async fn it_guards_replay_and_allows_an_explicit_retry() {
        let document = web_sys::window().unwrap().document().unwrap();
        let modal = document.create_element("section").unwrap();

        assert!(begin_action(&modal));
        assert!(!begin_action(&modal), "a second activation is ignored");
        modal.remove_attribute("data-running").unwrap();
        assert!(
            begin_action(&modal),
            "failure may re-enable an explicit retry"
        );
    }
}
