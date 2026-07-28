//! Top-document gate for durable operations that require a local root.

use std::cell::Cell;

use js_sys::{Function, Promise, Reflect};
use serde::Deserialize;
use tonk_worker_api::{IdentityIntent, IdentityRequired, RootStatus};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, Event, HtmlButtonElement, MessageEvent};

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootOutput {
    credential_id: String,
    delegation_hex: String,
}

fn show_concurrent_warning() {
    if let Some(document) = web_sys::window().and_then(|window| window.document())
        && let Some(status) = document.get_element_by_id("tonk-identity-status")
    {
        status.set_text_content(Some(
            "Finish the current identity request before starting another.",
        ));
    }
}

async fn identity_call(method: &str, device_did: &str) -> Result<RootOutput, String> {
    let window = web_sys::window().ok_or_else(|| "window is unavailable".to_string())?;
    let identity = Reflect::get(&window, &"tonkIdentity".into())
        .map_err(|_| "identity ceremonies are unavailable".to_string())?;
    let function: Function = Reflect::get(&identity, &method.into())
        .map_err(|_| format!("identity ceremony {method} is unavailable"))?
        .dyn_into()
        .map_err(|_| format!("identity ceremony {method} is not callable"))?;
    let input = serde_wasm_bindgen::to_value(&serde_json::json!({ "deviceDid": device_did }))
        .map_err(|error| error.to_string())?;
    let promise: Promise = function
        .call1(&identity, &input)
        .map_err(|error| format!("identity ceremony failed: {error:?}"))?
        .dyn_into()
        .map_err(|_| "identity ceremony did not return a promise".to_string())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|error| format!("identity ceremony failed: {error:?}"))?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
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
            template,
        } => {
            let response = client
                .post(format!("{origin}/api/spaces"))
                .json(&tonk_worker_api::CreateSpaceRequest {
                    name,
                    remote,
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
            if response.status().is_success() {
                Ok(())
            } else {
                Err(format!("operation failed with {}", response.status()))
            }
        }
    }
}

fn finish(modal: &Element) {
    modal.remove();
    ACTIVE.with(|active| active.set(false));
}

fn set_status(modal: &Element, message: &str) {
    if let Ok(Some(status)) = modal.query_selector("#tonk-identity-status") {
        status.set_text_content(Some(message));
    }
}

fn bind_link_action(
    button: HtmlButtonElement,
    method: &'static str,
    device_did: String,
    modal: Element,
) {
    let closure = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        let modal = modal.clone();
        let device_did = device_did.clone();
        set_status(&modal, "Waiting for your passkey…");
        spawn_local(async move {
            match identity_call(method, &device_did).await {
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
    bind_link_action(create, "createRoot", device_did.clone(), modal.clone());
    bind_link_action(existing, "evaluateRoot", device_did, modal);
    Ok(())
}

fn bind_action(
    button: HtmlButtonElement,
    method: &'static str,
    device_did: String,
    intent: IdentityIntent,
    modal: Element,
) {
    let closure = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        let modal = modal.clone();
        let device_did = device_did.clone();
        let intent = intent.clone();
        set_status(&modal, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let output = identity_call(method, &device_did).await?;
                crate::api::save_root(output.credential_id, output.delegation_hex)
                    .await
                    .map_err(|error| error.to_string())?;
                set_status(&modal, "Continuing…");
                replay(intent).await
            }
            .await;
            match result {
                Ok(()) => finish(&modal),
                Err(error) => set_status(&modal, &error),
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
    modal.set_inner_html(
        r#"<div class="tonk-identity-card">
<h2 id="tonk-identity-title">Create your local identity</h2>
<p>This durable action needs a passkey root stored on this device.</p>
<div class="tonk-identity-actions">
<button id="tonk-create-root" type="button">Create a new passkey</button>
<button id="tonk-use-root" type="button">Use an existing passkey</button>
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
    bind_action(
        create,
        "createRoot",
        device_did.clone(),
        intent.clone(),
        modal.clone(),
    );
    bind_action(existing, "evaluateRoot", device_did, intent, modal);
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
        if ACTIVE.with(|active| active.replace(true)) {
            show_concurrent_warning();
            return;
        }
        spawn_local(async move {
            if let Err(error) = show(message.intent).await {
                ACTIVE.with(|active| active.set(false));
                if let Some(window) = web_sys::window() {
                    let _ = window.alert_with_message(&error);
                }
            }
        });
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
    async fn it_mounts_create_and_use_existing_actions() {
        ACTIVE.with(|active| active.set(true));
        let document = web_sys::window().unwrap().document().unwrap();
        let modal = document.create_element("section").unwrap();
        modal.set_inner_html(
            r#"<button id="tonk-create-root">Create a new passkey</button>
<button id="tonk-use-root">Use an existing passkey</button>"#,
        );
        assert!(modal.query_selector("#tonk-create-root").unwrap().is_some());
        assert!(modal.query_selector("#tonk-use-root").unwrap().is_some());
        ACTIVE.with(|active| active.set(false));
    }
}
