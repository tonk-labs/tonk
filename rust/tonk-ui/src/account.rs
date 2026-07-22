//! Top-document account creation and passkey self-link surface.

use custom_elements::CustomElement;
use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, window};

use tonk_worker_api::AccountStatus;

const DEFAULT_ACCOUNT_SERVICE: &str = "https://accounts.tonk.spot";
const STYLE_ID: &str = "tonk-account-styles";
const PENDING_LINK: &str = "__tonkPendingAccountLink";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateInput {
    email: String,
    code: String,
    device_did: String,
    device_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkInput {
    device_did: String,
    device_name: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CeremonyOutput {
    root_did: String,
    delegation_hex: String,
    invocation_hex: String,
}

/// The top-document account element. WebAuthn must not run in sealed guests.
#[derive(Default)]
struct TonkAccount;

impl CustomElement for TonkAccount {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(include_str!("account.html"));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        ensure_stylesheet();
        if Reflect::get(this.as_ref(), &"__tonkAccountBound".into())
            .map(|value| value.is_truthy())
            .unwrap_or(false)
        {
            return;
        }
        let _ = Reflect::set(this.as_ref(), &"__tonkAccountBound".into(), &JsValue::TRUE);
        bind(this);
        load_status(this.clone());
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

fn ensure_stylesheet() {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(include_str!("account.css")));
    if let Some(head) = document.head() {
        let _ = head.append_child(&style);
    }
}

fn service(host: &HtmlElement) -> String {
    host.get_attribute("service")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_ACCOUNT_SERVICE.to_string())
}

fn input(host: &HtmlElement, selector: &str) -> Result<String, String> {
    let input: HtmlInputElement = host
        .query_selector(selector)
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into().ok())
        .ok_or_else(|| format!("missing form field {selector}"))?;
    let value = input.value().trim().to_string();
    if value.is_empty() {
        Err(format!("{} is required", input.name()))
    } else {
        Ok(value)
    }
}

fn set_mode(host: &HtmlElement, mode: &str) {
    let _ = host.set_attribute("data-mode", mode);
    for (name, selector) in [
        ("choice", "#account-choice"),
        ("create", "#account-create"),
        ("link", "#account-link"),
        ("success", "#account-success"),
    ] {
        if let Ok(Some(panel)) = host.query_selector(selector) {
            if name == mode {
                let _ = panel.remove_attribute("hidden");
            } else {
                let _ = panel.set_attribute("hidden", "");
            }
        }
    }
}

fn set_busy(host: &HtmlElement, busy: bool, status: &str) {
    for selector in [
        "#account-send-code",
        "#account-create-submit",
        "#account-link-submit",
        "#account-retry-local",
    ] {
        if let Ok(Some(button)) = host.query_selector(selector)
            && let Ok(button) = button.dyn_into::<HtmlButtonElement>()
        {
            button.set_disabled(busy);
        }
    }
    if let Ok(Some(element)) = host.query_selector("#account-working") {
        element.set_text_content((!status.is_empty()).then_some(status));
    }
}

fn show_error(host: &HtmlElement, message: impl AsRef<str>) {
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(Some(message.as_ref()));
        let _ = error.remove_attribute("hidden");
    }
}

fn clear_error(host: &HtmlElement) {
    if let Ok(Some(error)) = host.query_selector("#account-error") {
        error.set_text_content(None);
        let _ = error.set_attribute("hidden", "");
    }
}

fn show_success(host: &HtmlElement, root_did: &str) {
    clear_error(host);
    set_busy(host, false, "");
    if let Ok(Some(did)) = host.query_selector("#account-root-did") {
        did.set_text_content(Some(root_did));
    }
    if let Ok(Some(retry)) = host.query_selector("#account-retry-local") {
        let _ = retry.set_attribute("hidden", "");
    }
    set_mode(host, "success");
}

fn load_status(host: HtmlElement) {
    set_busy(&host, true, "Checking this browser…");
    spawn_local(async move {
        match crate::api::account_status().await {
            Ok(AccountStatus::Linked { root_did, .. }) => show_success(&host, &root_did),
            Ok(AccountStatus::Unlinked { .. }) => {
                set_busy(&host, false, "");
                set_mode(&host, "choice");
            }
            Err(error) => {
                set_busy(&host, false, "");
                set_mode(&host, "choice");
                show_error(&host, error.to_string());
            }
        }
    });
}

async fn identity_call<T: Serialize>(method: &str, input: &T) -> Result<CeremonyOutput, String> {
    let window = window().ok_or_else(|| "window is unavailable".to_string())?;
    let identity = Reflect::get(&window, &"tonkIdentity".into())
        .map_err(|error| format!("identity API unavailable: {error:?}"))?;
    let function: Function = Reflect::get(&identity, &method.into())
        .map_err(|error| format!("identity method unavailable: {error:?}"))?
        .dyn_into()
        .map_err(|_| format!("window.tonkIdentity.{method} is not a function"))?;
    let input = serde_wasm_bindgen::to_value(input).map_err(|error| error.to_string())?;
    let promise: Promise = function
        .call1(&identity, &input)
        .map_err(|error| format!("passkey ceremony failed: {error:?}"))?
        .dyn_into()
        .map_err(|_| "passkey ceremony did not return a promise".to_string())?;
    let output = JsFuture::from(promise)
        .await
        .map_err(|error| error.as_string().unwrap_or_else(|| format!("{error:?}")))?;
    serde_wasm_bindgen::from_value(output).map_err(|error| error.to_string())
}

fn save_pending(host: &HtmlElement, ceremony: &CeremonyOutput) {
    if let Ok(value) = serde_wasm_bindgen::to_value(ceremony) {
        let _ = Reflect::set(host.as_ref(), &PENDING_LINK.into(), &value);
    }
}

fn pending(host: &HtmlElement) -> Result<CeremonyOutput, String> {
    let value = Reflect::get(host.as_ref(), &PENDING_LINK.into())
        .map_err(|error| format!("failed to read pending link: {error:?}"))?;
    serde_wasm_bindgen::from_value(value).map_err(|_| "no account link is pending".to_string())
}

async fn persist(host: &HtmlElement, ceremony: &CeremonyOutput) -> Result<(), String> {
    save_pending(host, ceremony);
    crate::api::save_account_link(ceremony.root_did.clone(), ceremony.delegation_hex.clone())
        .await
        .map_err(|error| error.to_string())?;
    let _ = Reflect::delete_property(host.as_ref(), &PENDING_LINK.into());
    Ok(())
}

async fn complete_remote(
    host: &HtmlElement,
    path: &str,
    ceremony: CeremonyOutput,
) -> Result<(), String> {
    crate::api::submit_account_ceremony(&service(host), path, &ceremony.invocation_hex)
        .await
        .map_err(|error| error.to_string())?;
    persist(host, &ceremony).await.map_err(|error| {
        if let Ok(Some(retry)) = host.query_selector("#account-retry-local") {
            let _ = retry.remove_attribute("hidden");
        }
        format!("The account service accepted the link, but this browser could not save it. Retry the local save: {error}")
    })?;
    show_success(host, &ceremony.root_did);
    Ok(())
}

fn on_click(host: &HtmlElement, selector: &str, callback: impl Fn(HtmlElement) + 'static) {
    let Ok(Some(element)) = host.query_selector(selector) else {
        return;
    };
    let host = host.clone();
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        callback(host.clone());
    });
    let _ = element.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn bind(host: &HtmlElement) {
    on_click(host, "#account-choose-create", |host| {
        clear_error(&host);
        set_mode(&host, "create");
    });
    on_click(host, "#account-choose-link", |host| {
        clear_error(&host);
        set_mode(&host, "link");
    });
    for selector in ["#account-create-back", "#account-link-back"] {
        on_click(host, selector, |host| {
            clear_error(&host);
            set_mode(&host, "choice");
        });
    }

    on_click(host, "#account-send-code", |host| {
        clear_error(&host);
        let email = match input(&host, "#account-email") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        set_busy(&host, true, "Sending verification code…");
        spawn_local(async move {
            match crate::api::request_account_code(&service(&host), &email).await {
                Ok(()) => set_busy(&host, false, "Code sent. Check your email."),
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });

    on_click(host, "#account-create-submit", |host| {
        clear_error(&host);
        let fields = (
            input(&host, "#account-email"),
            input(&host, "#account-code"),
            input(&host, "#account-create-device-name"),
        );
        let (email, code, device_name) = match fields {
            (Ok(email), Ok(code), Ok(name)) => (email, code, name),
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                return show_error(&host, error);
            }
        };
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let device_did = crate::api::identify()
                    .await
                    .map_err(|error| error.to_string())?
                    .did;
                let ceremony = identity_call(
                    "createAccount",
                    &CreateInput {
                        email,
                        code,
                        device_did,
                        device_name,
                    },
                )
                .await?;
                set_busy(&host, true, "Creating your account…");
                complete_remote(&host, "/accounts", ceremony).await
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-link-submit", |host| {
        clear_error(&host);
        let device_name = match input(&host, "#account-link-device-name") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let device_did = crate::api::identify()
                    .await
                    .map_err(|error| error.to_string())?
                    .did;
                let ceremony = identity_call(
                    "linkDevice",
                    &LinkInput {
                        device_did,
                        device_name,
                    },
                )
                .await?;
                set_busy(&host, true, "Linking this browser…");
                complete_remote(&host, "/devices/link", ceremony).await
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-retry-local", |host| {
        clear_error(&host);
        set_busy(&host, true, "Saving the link locally…");
        spawn_local(async move {
            let result = match pending(&host) {
                Ok(ceremony) => persist(&host, &ceremony).await.map(|()| ceremony.root_did),
                Err(error) => Err(error),
            };
            match result {
                Ok(root_did) => show_success(&host, &root_did),
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error);
                }
            }
        });
    });
}

/// Register `<tonk-account>` with the top document.
pub fn register() {
    if let Some(window) = window()
        && window.custom_elements().get("tonk-account").is_undefined()
    {
        TonkAccount::define("tonk-account");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn host() -> HtmlElement {
        let host: HtmlElement = window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("tonk-account")
            .unwrap()
            .unchecked_into();
        let mut element = TonkAccount;
        element.inject_children(&host);
        host
    }

    #[dialog_common::test]
    fn it_authors_the_create_and_self_link_controls() {
        let host = host();
        for selector in [
            "#account-send-code",
            "#account-create-submit",
            "#account-link-submit",
            "#account-retry-local",
        ] {
            assert!(
                host.query_selector(selector).unwrap().is_some(),
                "{selector}"
            );
        }
    }

    #[dialog_common::test]
    fn it_switches_between_account_panels_without_reauthoring_the_dom() {
        let host = host();
        set_mode(&host, "link");
        assert!(
            host.query_selector("#account-link")
                .unwrap()
                .unwrap()
                .get_attribute("hidden")
                .is_none()
        );
        assert!(
            host.query_selector("#account-create")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
    }
}
