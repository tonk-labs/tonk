//! Top-document account creation and passkey self-link surface.

use custom_elements::CustomElement;
use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{HtmlButtonElement, HtmlElement, HtmlInputElement, window};

use tonk_worker_api::AccountStatus;

const PROD: &str = "https://accounts.tonk.xyz";
const STAGING: &str = "https://accounts-staging.tonk.xyz";
const STYLE_ID: &str = "tonk-account-styles";
const HANDOFF: &str = "__tonkCliHandoff";

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
struct HandoffInput {
    token_hash: String,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RotateInput {
    name: String,
    device_did: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RotationOutput {
    #[allow(dead_code)]
    old_root_did: String,
    new_root_did: String,
    #[allow(dead_code)]
    new_credential_id: String,
    succession_hex: String,
    device_delegation_hex: String,
    rotation_hex: String,
    confirmation_hex: String,
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

/// The account service for a page host, or `None` if account ceremonies
/// must not run there.
///
/// Refuse-by-default, not production-by-default. Ceremonies run only where
/// the host is the pinned apex or is its own relying party by design (see
/// `tonk_identity::passkey`), so that one user has exactly one root key per
/// environment. Widening `_` re-opens that: `hub.tonk.xyz` serves the same
/// production build but is a different relying party, so a ceremony there
/// would write a second, disjoint identity into the production registry.
fn default_service(host: &str) -> Option<&'static str> {
    match host {
        "tonk.spot" => Some(PROD),
        "staging.tonk.xyz" => Some(STAGING),
        _ => None,
    }
}

fn service(host: &HtmlElement) -> Result<String, String> {
    if let Some(attribute) = host
        .get_attribute("service")
        .filter(|value| !value.is_empty())
    {
        return Ok(attribute);
    }
    let hostname = window()
        .and_then(|window| window.location().hostname().ok())
        .ok_or_else(|| "window is unavailable".to_string())?;
    default_service(&hostname)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("Accounts are not available on {hostname}. Go to https://tonk.spot/account.")
        })
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
        ("verify", "#account-verify"),
        ("link", "#account-link"),
        ("handoff", "#account-handoff"),
        ("success", "#account-success"),
        ("devices", "#account-devices"),
        ("rotate", "#account-rotate"),
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
        "#account-handoff-submit",
        "#account-manage-devices",
        "#account-unlink",
        "#account-rotate-submit",
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

fn focus_input(host: &HtmlElement, selector: &str) {
    if let Ok(Some(input)) = host.query_selector(selector)
        && let Ok(input) = input.dyn_into::<HtmlInputElement>()
    {
        let _ = input.focus();
    }
}

fn show_success(host: &HtmlElement) {
    clear_error(host);
    set_busy(host, false, "");
    set_mode(host, "success");
}

fn show_handoff_success(host: &HtmlElement) {
    if let Ok(Some(message)) = host.query_selector("#account-success-message") {
        message.set_text_content(Some("The command-line profile is connected."));
    }
    show_success(host);
}

fn render_devices(host: &HtmlElement, devices: &[tonk_worker_api::AccountDevice]) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector("#account-device-list") else {
        return;
    };
    list.set_inner_html("");
    for device in devices {
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("class", "account__device-row");

        let Ok(name) = document.create_element("span") else {
            continue;
        };
        name.set_text_content(Some(&device.name));

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__device-meta");
        let registered = js_sys::Date::new(&JsValue::from_f64(device.created_at as f64 * 1000.0))
            .to_locale_date_string("default", &JsValue::UNDEFINED);
        let mut details = format!("{} · {}", device.status, String::from(registered));
        if device.this_device {
            details.push_str(" · this device");
        }
        meta.set_text_content(Some(&details));

        let _ = item.append_child(&name);
        let _ = item.append_child(&meta);

        if device.status == "active" && !device.this_device {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("class", "account__button account__button--quiet");
            let _ = button.set_attribute("data-revoke", &device.did);
            button.set_text_content(Some("Revoke"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

fn load_devices(host: HtmlElement) {
    set_busy(&host, true, "Loading devices…");
    spawn_local(async move {
        match crate::api::account_devices().await {
            Ok(devices) => {
                set_busy(&host, false, "");
                render_devices(&host, &devices);
                set_mode(&host, "devices");
            }
            Err(error) => {
                set_busy(&host, false, "");
                show_error(&host, error.to_string());
            }
        }
    });
}

/// The account rotated server-side, but this browser failed to persist the
/// new link. Send the user to log in with the new passkey rather than
/// leaving the rotate panel (and its "create a new passkey" submit) live,
/// which would risk a third credential.
fn show_relink_failure(host: &HtmlElement) {
    set_busy(host, false, "");
    set_mode(host, "link");
    show_error(
        host,
        "Your account is now on the new passkey, but this browser could not \
         finish linking. Do not create another passkey — use Log in with your \
         new passkey instead.",
    );
}

fn load_status(host: HtmlElement) {
    if let Err(error) = service(&host) {
        set_mode(&host, "blocked");
        return show_error(&host, error);
    }
    let handoff_route = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/account/link" || path.starts_with("/account/link/"));
    if handoff_route {
        load_handoff(host);
        return;
    }
    set_busy(&host, true, "Checking this browser…");
    spawn_local(async move {
        match crate::api::account_status().await {
            Ok(AccountStatus::Linked { .. }) => show_success(&host),
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

fn load_handoff(host: HtmlElement) {
    let Some(window) = window() else {
        return show_error(&host, "window is unavailable");
    };
    let secret = window
        .location()
        .hash()
        .ok()
        .and_then(|hash| hash.strip_prefix('#').map(str::to_owned))
        .filter(|secret| !secret.is_empty());
    let Some(secret) = secret else {
        set_mode(&host, "handoff");
        return show_error(
            &host,
            "This link is missing its handoff secret. Start again from the terminal.",
        );
    };
    if let Ok(path) = window.location().pathname() {
        let _ = window
            .history()
            .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&path)));
    }

    let service_url = match service(&host) {
        Ok(service_url) => service_url,
        Err(error) => {
            set_mode(&host, "handoff");
            return show_error(&host, error);
        }
    };
    set_busy(&host, true, "Checking the command-line request…");
    spawn_local(async move {
        match crate::api::resolve_account_link(&service_url, &secret).await {
            Ok(link) => {
                let handoff = HandoffInput {
                    token_hash: link.token_hash,
                    device_did: link.device_did,
                    device_name: link.device_name,
                };
                if let Ok(value) = serde_wasm_bindgen::to_value(&handoff) {
                    let _ = Reflect::set(host.as_ref(), &HANDOFF.into(), &value);
                }
                if let Ok(Some(name)) = host.query_selector("#account-handoff-name") {
                    name.set_text_content(Some(&handoff.device_name));
                }
                if let Ok(Some(did)) = host.query_selector("#account-handoff-did") {
                    did.set_text_content(Some(&handoff.device_did));
                }
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
            }
            Err(error) => {
                set_busy(&host, false, "");
                set_mode(&host, "handoff");
                show_error(&host, error.to_string());
            }
        }
    });
}

async fn identity_call<I: Serialize, O: for<'de> Deserialize<'de>>(
    method: &str,
    input: &I,
) -> Result<O, String> {
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

async fn persist(ceremony: &CeremonyOutput) -> Result<(), String> {
    crate::api::save_account_link(
        ceremony.root_did.clone(),
        ceremony.delegation_hex.clone(),
        None,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn complete_remote(
    host: &HtmlElement,
    path: &str,
    ceremony: CeremonyOutput,
) -> Result<(), String> {
    crate::api::submit_account_ceremony(&service(host)?, path, &ceremony.invocation_hex)
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = persist(&ceremony).await {
        web_sys::console::error_1(
            &format!("failed to save the accepted account link: {error}").into(),
        );
        set_mode(host, "choice");
        return Err(
            "Your account is ready, but this browser couldn't finish signing in. Log in to continue."
                .to_string(),
        );
    }
    show_success(host);
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
        focus_input(&host, "#account-email");
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
    on_click(host, "#account-verify-back", |host| {
        clear_error(&host);
        set_busy(&host, false, "");
        set_mode(&host, "create");
        focus_input(&host, "#account-email");
    });

    on_click(host, "#account-send-code", |host| {
        clear_error(&host);
        let email = match input(&host, "#account-email") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        let service_url = match service(&host) {
            Ok(service_url) => service_url,
            Err(error) => return show_error(&host, error),
        };
        set_busy(&host, true, "Sending verification code…");
        spawn_local(async move {
            match crate::api::request_account_code(&service_url, &email).await {
                Ok(()) => {
                    set_busy(&host, false, "");
                    if let Ok(Some(destination)) = host.query_selector("#account-code-email") {
                        destination.set_text_content(Some(&email));
                    }
                    set_mode(&host, "verify");
                    if let Ok(Some(code)) = host.query_selector("#account-code")
                        && let Ok(code) = code.dyn_into::<HtmlInputElement>()
                    {
                        code.set_value("");
                        let _ = code.focus();
                    }
                }
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
                let ceremony: CeremonyOutput = identity_call(
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
                let ceremony: CeremonyOutput = identity_call(
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

    on_click(host, "#account-handoff-submit", |host| {
        clear_error(&host);
        let handoff = Reflect::get(host.as_ref(), &HANDOFF.into())
            .ok()
            .and_then(|value| serde_wasm_bindgen::from_value::<HandoffInput>(value).ok());
        let Some(handoff) = handoff else {
            return show_error(
                &host,
                "This handoff is no longer available. Start again from the terminal.",
            );
        };
        set_busy(&host, true, "Waiting for your passkey…");
        spawn_local(async move {
            let result = async {
                let ceremony: CeremonyOutput = identity_call("completeLink", &handoff).await?;
                set_busy(&host, true, "Linking the command-line profile…");
                crate::api::submit_account_ceremony(
                    &service(&host)?,
                    "/links/complete",
                    &ceremony.invocation_hex,
                )
                .await
                .map_err(|error| error.to_string())?;
                show_handoff_success(&host);
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = result {
                set_busy(&host, false, "");
                show_error(&host, error);
            }
        });
    });

    on_click(host, "#account-manage-devices", |host| {
        clear_error(&host);
        load_devices(host);
    });
    on_click(host, "#account-devices-back", |host| {
        clear_error(&host);
        set_mode(&host, "success");
    });
    on_click(host, "#account-unlink", |host| {
        clear_error(&host);
        let confirmed = window()
            .map(|window| {
                window
                    .confirm_with_message(
                        "Sign out of your account on this device? Your data stays; \
                         this browser stops acting as the account until you log in again.",
                    )
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        set_busy(&host, true, "Signing out…");
        spawn_local(async move {
            match crate::api::unlink_account().await {
                Ok(_) => {
                    set_busy(&host, false, "");
                    set_mode(&host, "choice");
                }
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });

    if let Ok(Some(list)) = host.query_selector("#account-device-list") {
        let host_for_revoke = host.clone();
        let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(did) = target.get_attribute("data-revoke") else {
                return;
            };
            let host = host_for_revoke.clone();
            let confirmed = window()
                .map(|window| {
                    window
                        .confirm_with_message(
                            "Revoke this device? It immediately loses account and sync \
                             access. Spaces it joined before it was linked may need a \
                             fresh invite.",
                        )
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if !confirmed {
                return;
            }
            clear_error(&host);
            set_busy(&host, true, "Revoking device…");
            spawn_local(async move {
                match crate::api::revoke_account_device(did).await {
                    Ok(devices) => {
                        set_busy(&host, false, "");
                        render_devices(&host, &devices);
                    }
                    Err(error) => {
                        set_busy(&host, false, "");
                        show_error(&host, error.to_string());
                    }
                }
            });
        });
        let _ = list.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    on_click(host, "#account-rotate-open", |host| {
        clear_error(&host);
        set_mode(&host, "rotate");
        focus_input(&host, "#account-rotate-name");
    });

    on_click(host, "#account-rotate-submit", |host| {
        clear_error(&host);
        let name = match input(&host, "#account-rotate-name") {
            Ok(value) => value,
            Err(error) => return show_error(&host, error),
        };
        set_busy(&host, true, "Waiting for your current passkey…");
        spawn_local(async move {
            let setup = async {
                let device_did = crate::api::identify()
                    .await
                    .map_err(|error| error.to_string())?
                    .did;
                let rotation: RotationOutput =
                    identity_call("rotateAccount", &RotateInput { name, device_did }).await?;
                let service_url = service(&host)?;
                Ok::<(String, RotationOutput), String>((service_url, rotation))
            }
            .await;
            let (service_url, rotation) = match setup {
                Ok(value) => value,
                Err(error) => {
                    set_busy(&host, false, "");
                    return show_error(&host, error);
                }
            };

            set_busy(&host, true, "Rotating your account…");
            if let Err(error) = crate::api::rotate_account(
                &service_url,
                &rotation.rotation_hex,
                &rotation.confirmation_hex,
            )
            .await
            {
                // Nothing changed server-side: safe to retry the whole rotation.
                set_busy(&host, false, "");
                return show_error(&host, error.to_string());
            }

            match crate::api::save_account_link(
                rotation.new_root_did,
                rotation.device_delegation_hex,
                Some(&rotation.succession_hex),
            )
            .await
            {
                Ok(_status) => show_success(&host),
                Err(error) => {
                    web_sys::console::error_1(
                        &format!("failed to save the rotated account link: {error}").into(),
                    );
                    show_relink_failure(&host);
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

    /// Yield to the event loop for `ms` milliseconds.
    async fn yield_for(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let win = window().unwrap();
            win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    /// Build a `<tonk-account>` host with its panels injected, attach it to
    /// the document body so it sits in a real DOM tree, and give it a tick
    /// to settle.
    async fn mounted_account_host() -> HtmlElement {
        let host = host();
        window()
            .unwrap()
            .document()
            .unwrap()
            .body()
            .unwrap()
            .append_child(host.as_ref())
            .unwrap();
        yield_for(0).await;
        host
    }

    #[dialog_common::test]
    fn it_authors_the_create_and_self_link_controls() {
        let host = host();
        for selector in [
            "#account-send-code",
            "#account-create-submit",
            "#account-link-submit",
            "#account-handoff-submit",
            "#account-rotate-open",
            "#account-rotate-name",
            "#account-rotate-submit",
        ] {
            assert!(
                host.query_selector(selector).unwrap().is_some(),
                "{selector}"
            );
        }
        assert_eq!(
            host.query_selector("#account-choose-link")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Log in")
        );
        assert!(
            host.query_selector("#account-retry-local")
                .unwrap()
                .is_none(),
            "local persistence recovery must not be exposed in the account UI"
        );
    }

    #[dialog_common::test]
    fn it_switches_between_account_panels_without_reauthoring_the_dom() {
        let host = host();
        set_mode(&host, "verify");
        assert!(
            host.query_selector("#account-verify")
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
        assert!(
            host.query_selector("#account-create #account-code")
                .unwrap()
                .is_none(),
            "email and verification fields should be on separate screens"
        );
    }

    #[dialog_common::test]
    fn it_reveals_the_rotate_panel_and_hides_the_others() {
        let host = host();
        set_mode(&host, "rotate");
        assert!(
            host.query_selector("#account-rotate")
                .unwrap()
                .unwrap()
                .get_attribute("hidden")
                .is_none()
        );
        for selector in [
            "#account-choice",
            "#account-create",
            "#account-verify",
            "#account-link",
            "#account-handoff",
            "#account-success",
        ] {
            assert!(
                host.query_selector(selector)
                    .unwrap()
                    .unwrap()
                    .has_attribute("hidden"),
                "{selector} should be hidden while rotate is shown"
            );
        }
    }

    #[dialog_common::test]
    fn it_routes_a_failed_post_rotation_relink_to_log_in() {
        let host = host();
        set_mode(&host, "rotate");
        set_busy(&host, true, "Rotating your account…");

        show_relink_failure(&host);

        assert!(
            host.query_selector("#account-link")
                .unwrap()
                .unwrap()
                .get_attribute("hidden")
                .is_none(),
            "the log-in panel should be shown"
        );
        assert!(
            host.query_selector("#account-rotate")
                .unwrap()
                .unwrap()
                .has_attribute("hidden"),
            "the rotate panel (with its create-a-new-passkey submit) must not stay live"
        );
        assert_eq!(
            host.query_selector("#account-error")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some(
                "Your account is now on the new passkey, but this browser could not \
                 finish linking. Do not create another passkey — use Log in with your \
                 new passkey instead."
            )
        );
        assert!(
            !host
                .query_selector("#account-rotate-submit")
                .unwrap()
                .unwrap()
                .unchecked_into::<HtmlButtonElement>()
                .disabled(),
            "buttons must be re-enabled, not left busy"
        );
    }

    #[dialog_common::test]
    fn it_maps_each_environment_host_to_its_own_service() {
        assert_eq!(default_service("tonk.spot"), Some(PROD));
        assert_eq!(default_service("staging.tonk.xyz"), Some(STAGING));
    }

    #[dialog_common::test]
    fn it_refuses_ceremonies_on_unmapped_hosts() {
        // Off-apex, so it is its own relying party: a ceremony here would
        // derive a different root key and write a second identity for the
        // same person into the production registry.
        assert_eq!(default_service("hub.tonk.xyz"), None);
        // Inside the apex but not the apex origin, so also its own RP.
        assert_eq!(default_service("staging.tonk.spot"), None);
        assert_eq!(default_service("www.tonk.spot"), None);
        assert_eq!(default_service("random123.tonk.spot"), None);
        assert_eq!(default_service("localhost"), None);
    }

    #[dialog_common::test]
    fn it_prefers_an_explicit_service_attribute_over_the_host() {
        let host = host();
        host.set_attribute("service", "http://127.0.0.1:8787")
            .unwrap();
        assert_eq!(service(&host).unwrap(), "http://127.0.0.1:8787");
    }

    #[dialog_common::test]
    fn it_errors_when_the_host_has_no_mapping_and_no_attribute() {
        // wasm tests run on a localhost origin, which is unmapped.
        let host = host();
        assert!(service(&host).is_err());
    }

    #[dialog_common::test]
    async fn it_renders_the_device_list_with_a_this_device_marker() {
        let host = mounted_account_host().await;
        let devices = vec![
            tonk_worker_api::AccountDevice {
                did: "did:key:zThis".into(),
                name: "This browser".into(),
                status: "active".into(),
                created_at: 1_753_300_000,
                this_device: true,
            },
            tonk_worker_api::AccountDevice {
                did: "did:key:zOther".into(),
                name: "Old laptop".into(),
                status: "revoked".into(),
                created_at: 1_753_200_000,
                this_device: false,
            },
            tonk_worker_api::AccountDevice {
                did: "did:key:zPhone".into(),
                name: "Phone".into(),
                status: "active".into(),
                created_at: 1_753_100_000,
                this_device: false,
            },
        ];
        render_devices(&host, &devices);

        let list = host
            .query_selector("#account-device-list")
            .unwrap()
            .unwrap();
        let items = list.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 3);
        let text = list.text_content().unwrap();
        assert!(text.contains("This browser"));
        assert!(text.contains("this device"));
        assert!(text.contains("revoked"));
        // Only the active, non-self row gets a revoke button.
        assert_eq!(
            list.query_selector_all("button[data-revoke]")
                .unwrap()
                .length(),
            1
        );
    }
}
