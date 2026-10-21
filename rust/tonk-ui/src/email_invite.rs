//! The emailed locator remains on the trusted page throughout signup and claim.
use custom_elements::CustomElement;
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{HtmlButtonElement, HtmlElement};

/// Accept only the exact local invitation route and opaque locator.
pub(crate) fn destination(value: &str) -> bool {
    let Some(id) = value.strip_prefix("/email-invite#") else {
        return false;
    };
    valid_id(id)
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Default)]
struct EmailInvite;
impl CustomElement for EmailInvite {
    fn shadow() -> bool {
        false
    }
    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }
    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(include_str!("email_invite.html"));
    }
    fn connected_callback(&mut self, this: &HtmlElement) {
        if this.has_attribute("data-bound") {
            return;
        }
        let _ = this.set_attribute("data-bound", "");
        if let Some(doc) = this.owner_document()
            && doc.get_element_by_id("tonk-activate-styles").is_none()
            && let Ok(style) = doc.create_element("style")
        {
            style.set_id("tonk-activate-styles");
            style.set_text_content(Some(include_str!("account.css")));
            if let Some(head) = doc.head() {
                let _ = head.append_child(&style);
            }
        }
        let id = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .unwrap_or_default();
        let id = id.trim_start_matches('#').to_owned();
        if !valid_id(&id) {
            status(
                this,
                "This invitation link is incomplete. Open the exact link from your email.",
            );
            for selector in ["#email-invite-account", "#email-invite-join"] {
                if let Some(button) = button(this, selector) {
                    button.set_disabled(true);
                }
            }
            return;
        }
        if let Some(button) = button(this, "#email-invite-account") {
            let callback = Closure::<dyn FnMut()>::new(move || {
                wasm_bindgen_futures::spawn_local(async {
                    resolve_profile().await;
                    crate::register_dialog::open();
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
        if let Some(button) = button(this, "#email-invite-join") {
            let host = this.clone();
            let action = button.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if action.disabled() {
                    return;
                }
                action.set_disabled(true);
                status(&host, "Checking your invitation…");
                let id = web_sys::window()
                    .and_then(|w| w.location().hash().ok())
                    .unwrap_or_default();
                let id = id.trim_start_matches('#').to_owned();
                let (host, action) = (host.clone(), action.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(message) = claim(&host, &id).await {
                        status(&host, &message);
                    }
                    action.set_disabled(false);
                });
            });
            button.set_onclick(Some(callback.as_ref().unchecked_ref()));
            callback.forget();
        }
    }
    fn disconnected_callback(&mut self, _: &HtmlElement) {}
    fn attribute_changed_callback(
        &mut self,
        _: &HtmlElement,
        _: String,
        _: Option<String>,
        _: Option<String>,
    ) {
    }
}

/// Register the invitation page element.
pub fn register() {
    EmailInvite::define("tonk-email-invite");
}
fn button(host: &HtmlElement, selector: &str) -> Option<HtmlButtonElement> {
    host.query_selector(selector)
        .ok()
        .flatten()?
        .dyn_into()
        .ok()
}
fn status(host: &HtmlElement, message: &str) {
    if let Ok(Some(node)) = host.query_selector("#email-invite-status") {
        node.set_text_content(Some(message));
    }
}

async fn claim(host: &HtmlElement, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(
            "This invitation link is incomplete. Open the exact link from your email.".into(),
        );
    }
    resolve_profile().await;
    let root = crate::api::root_status()
        .await
        .map_err(|_| "Couldn’t read your account. Try again.")?;
    let tonk_worker_api::RootStatus::Ready { root_did, .. } = root else {
        return Err("Create an account or sign in using the email that received this invitation, then select Join Space.".into());
    };
    let request = format!(
        "request:email-invite-{}-{}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * u64::MAX as f64) as u64
    );
    let branch = crate::api::profile_branch();
    let field = |the: &str, kind: &str| json!({"the":the,"as":kind,"cardinality":"one"});
    crate::api::transact_profile(json!({"claims":[{"op":"assert","application":{
        "predicate":{"kind":"transient","concept":{"with":{
            "id":field("xyz.tonk.claim-email-invitation/id","Text"),
            "account":field("xyz.tonk.claim-email-invitation/account","Entity")
        }}},"parameters":{"this":request,"id":id,"account":root_did}
    }}]}))
    .await
    .map_err(|_| "Couldn’t start joining. Your invitation is safe to retry.")?;
    let query = json!({"predicate":{"with":{
        "state":field("xyz.tonk.email-invitation-status/state","Text"),
        "message":field("xyz.tonk.email-invitation-status/message","Text")
    }},"terms":{"this":request,"state":{"?":{"name":"state"}},"message":{"?":{"name":"message"}}}});
    // Read the command's overlay outcome. No claim is replayed by this wait.
    for _ in 0..60 {
        if !host.is_connected() {
            return Ok(());
        }
        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/repository/profile:tonk/branch/{branch}/query",
                crate::api::origin()
            ))
            .json(&query)
            .send()
            .await
            .map_err(|_| "Couldn’t read the result. Select Join Space to retry safely.")?;
        if !response.status().is_success() {
            return Err("Couldn’t read the result. Select Join Space to retry safely.".into());
        }
        let rows: Vec<Value> = response
            .json()
            .await
            .map_err(|_| "Couldn’t read the invitation result.")?;
        if let Some(fields) = rows
            .iter()
            .map(|r| &r["fields"])
            .find(|r| r["state"] != "pending")
        {
            let message = fields["message"]
                .as_str()
                .unwrap_or("Couldn’t join this space.");
            if fields["state"] == "joined" {
                status(host, message);
                return Ok(());
            }
            return Err(message.into());
        }
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ =
                    window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 1000);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
    Err("Joining is taking longer than expected. Select Join Space to retry safely.".into())
}

async fn resolve_profile() {
    #[cfg(target_arch = "wasm32")]
    tonk_host::bridge::resolve_profile_with().await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn signup_return_is_only_an_exact_local_locator() {
        assert!(super::destination(&format!(
            "/email-invite#{}",
            "a".repeat(64)
        )));
        for value in [
            "//evil.test/email-invite#abc",
            "/email-invite#abc",
            "/email-invite?next=//evil.test#abc",
            "/email-invite/../other#abc",
        ] {
            assert!(!super::destination(value));
        }
    }
}
