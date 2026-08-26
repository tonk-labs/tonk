//! Top-document customer activation surface.
//!
//! The activation email links here with `?ucan=<base64url>`: a complete,
//! service-signed `/customer/activate` invocation. Presenting it is
//! activating, so this page needs no key and works on any device; the
//! accept button posts the decoded bytes to the same-origin `/ucan/`
//! endpoint and reports the outcome.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlElement, window};

const STYLE_ID: &str = "tonk-activate-styles";

/// The top-document activation element.
#[derive(Default)]
struct TonkActivate;

impl CustomElement for TonkActivate {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(include_str!("activate.html"));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        ensure_stylesheet();
        if Reflect::get(this.as_ref(), &"__tonkActivateBound".into())
            .map(|value| value.is_truthy())
            .unwrap_or(false)
        {
            return;
        }
        let _ = Reflect::set(this.as_ref(), &"__tonkActivateBound".into(), &JsValue::TRUE);
        bind(this);
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

/// Register `<tonk-activate>`.
pub fn register() {
    TonkActivate::define("tonk-activate");
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

/// The invocation bytes the link carries, from `?ucan=<base64url>`.
fn link_invocation() -> Result<Vec<u8>, String> {
    let search = window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let encoded = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("ucan="))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "This activation link is incomplete. Open the exact link from your email.".to_string()
        })?;
    URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        "This activation link is damaged. Open the exact link from your email.".to_string()
    })
}

fn set_status(host: &HtmlElement, message: &str) {
    if let Ok(Some(status)) = host.query_selector("#activate-working") {
        status.set_text_content(Some(message));
    }
}

fn show_error(host: &HtmlElement, message: &str) {
    if let Ok(Some(error)) = host.query_selector("#activate-error") {
        error.set_text_content(Some(message));
        let _ = error.remove_attribute("hidden");
    }
    set_status(host, "");
}

fn show_panel(host: &HtmlElement, selector: &str) {
    for panel in ["#activate-confirm", "#activate-done"] {
        if let Ok(Some(element)) = host.query_selector(panel) {
            let outcome = if panel == selector {
                element.remove_attribute("hidden")
            } else {
                element.set_attribute("hidden", "")
            };
            let _ = outcome;
        }
    }
}

fn bind(host: &HtmlElement) {
    // A damaged link is visible before the button is pressed rather than
    // after: the page's one job is presenting the invocation it carries.
    if let Err(error) = link_invocation() {
        show_error(host, &error);
        return;
    }
    show_panel(host, "#activate-confirm");

    let target = host.clone();
    let onclick = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        let host = target.clone();
        spawn_local(async move {
            let invocation = match link_invocation() {
                Ok(bytes) => bytes,
                Err(error) => return show_error(&host, &error),
            };
            set_status(&host, "Activating…");
            let response = reqwest::Client::new()
                .post(format!("{}/ucan/", crate::api::origin()))
                .header("content-type", "application/cbor")
                .body(invocation)
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    return show_error(&host, "The service could not be reached. Try again.");
                }
            };
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            if status.is_success() {
                // Hand the receipt to the worker before declaring
                // success. The service names the provider serving this
                // account here, and this page is the only place that
                // answer arrives — post it to `/api/customer/activated`
                // so it is recorded as a fact rather than discarded.
                // Best effort: activation itself succeeded, and the
                // status probe records the same thing on the next read.
                if let Err(error) = crate::api::report_activation(&body).await {
                    web_sys::console::warn_1(
                        &format!("activation receipt not recorded: {error}").into(),
                    );
                }
                set_status(&host, "");
                show_panel(&host, "#activate-done");
            } else if body["error"]["code"].as_str() == Some("Unauthorized") {
                show_error(
                    &host,
                    "This activation link has expired. Sign in on your device to get a fresh one.",
                );
            } else {
                let message = body["error"]["message"]
                    .as_str()
                    .unwrap_or("Activation failed. Try again.")
                    .to_string();
                show_error(&host, &message);
            }
        });
    });
    if let Ok(Some(button)) = host.query_selector("#activate-accept")
        && let Ok(button) = button.dyn_into::<HtmlElement>()
    {
        button.set_onclick(Some(onclick.as_ref().unchecked_ref()));
    }
    onclick.forget();
}
