//! Invitation listing and revocation controls.
//!
//! Minting is not here. The bar mints one kind of invitation — the open link
//! behind the share control (`crate::share`) — and revocation is what this
//! panel is for: every invitation this spot has issued, with a Revoke action
//! on the active ones. Minting to a named root DID reached the worker and the
//! CLI first; the browser control for it is deferred rather than shipped
//! half-designed, so nothing here asks for a pasted DID.

use custom_elements::CustomElement;
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlElement, window};

use tonk_worker_api::{InvitationKind, InvitationSummary, RevokeInvitationAcknowledgement};

use crate::logic::{invitations_endpoint, revoke_invitation_endpoint};

#[derive(Default)]
pub(crate) struct TonkInvitations {
    click: Option<Closure<dyn FnMut(Event)>>,
}

impl CustomElement for TonkInvitations {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_inner_html(
            r#"<p class="fab__invitation-status" role="status" aria-live="polite"></p>
<ul class="fab__invitation-list"></ul>"#,
        );
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let click_host = this.clone();
        let click = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(cid) = target.get_attribute("data-revoke-invitation") else {
                return;
            };
            event.prevent_default();
            revoke(click_host.clone(), cid);
        });
        let _ = this.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        self.click = Some(click);
        refresh(this.clone());
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
        refresh(this.clone());
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(click) = self.click.take() {
            let _ =
                this.remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        }
    }
}

fn origin() -> Option<String> {
    tonk_host::bridge::context_origin()
}

fn status(host: &HtmlElement, message: &str) {
    if let Ok(Some(element)) = host.query_selector(".fab__invitation-status") {
        element.set_text_content(Some(message));
    }
}

fn refresh(host: HtmlElement) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(path) = invitations_endpoint(&space) else {
        return;
    };
    let Some(origin) = origin() else {
        return;
    };
    spawn_local(async move {
        let result = reqwest::Client::new()
            .get(format!("{origin}{path}"))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => {
                match response.json::<Vec<InvitationSummary>>().await {
                    Ok(rows) => render(&host, &rows),
                    Err(_) => status(&host, "Invitation list is unavailable."),
                }
            }
            _ => status(&host, "Invitation list is unavailable."),
        }
    });
}

fn revoke(host: HtmlElement, target_cid: String) {
    let Some(space) = host.get_attribute("space") else {
        return;
    };
    let Ok(path) = revoke_invitation_endpoint(&space, &target_cid) else {
        return;
    };
    let Some(origin) = origin() else {
        return;
    };
    status(&host, "Revoking invitation…");
    spawn_local(async move {
        let result = reqwest::Client::new()
            .post(format!("{origin}{path}"))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => {
                match response.json::<RevokeInvitationAcknowledgement>().await {
                    Ok(acknowledgement) if acknowledgement.published => {
                        status(
                            &host,
                            if acknowledgement.stored {
                                "Invitation revoked."
                            } else {
                                "Invitation was already revoked."
                            },
                        );
                        mark_revoked(&host, &target_cid);
                    }
                    _ => status(&host, "The relay did not confirm publication."),
                }
            }
            Ok(response) => status(
                &host,
                &format!("Could not revoke invitation ({}).", response.status()),
            ),
            Err(_) => status(&host, "Could not revoke invitation."),
        }
    });
}

fn render(host: &HtmlElement, rows: &[InvitationSummary]) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector(".fab__invitation-list") else {
        return;
    };
    list.set_inner_html("");
    for row in rows {
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("data-invitation-cid", &row.target_cid);
        let recipient = row
            .recipient_root
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| match row.kind {
                InvitationKind::Open => "open invitation".to_string(),
                InvitationKind::Scoped => "scoped invitation".to_string(),
                InvitationKind::Unknown => "legacy invitation".to_string(),
            });
        let Ok(label) = document.create_element("span") else {
            continue;
        };
        label.set_text_content(Some(&format!("{recipient} · {}", row.status)));
        let _ = item.append_child(&label);
        if row.status == "active" {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("data-revoke-invitation", &row.target_cid);
            button.set_text_content(Some("Revoke"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}

fn mark_revoked(host: &HtmlElement, target_cid: &str) {
    let selector = format!("[data-invitation-cid=\"{target_cid}\"]");
    if let Ok(Some(item)) = host.query_selector(&selector) {
        if let Ok(Some(button)) = item.query_selector("[data-revoke-invitation]") {
            button.remove();
        }
        let existing = item.text_content().unwrap_or_default();
        item.set_text_content(Some(&format!("{existing} · revoked")));
    }
}

pub(crate) fn register() {
    let Some(elements) = window().map(|window| window.custom_elements()) else {
        return;
    };
    if elements.get("tonk-invitations").is_undefined() {
        TonkInvitations::define("tonk-invitations");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn injected() -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("div")
            .expect("create host")
            .unchecked_into();
        TonkInvitations::default().inject_children(&host);
        host
    }

    /// Minting to a named root is deferred. Any control left behind is a live
    /// entry point to a flow nothing else in the UI reaches — and it asks for
    /// a pasted DID, which is not a thing to leave lying around half-wired.
    #[dialog_common::test]
    fn it_offers_no_targeted_minting_control() {
        let host = injected();
        assert!(
            host.query_selector(".fab__target-invite")
                .expect("query")
                .is_none(),
            "the panel must mint nothing",
        );
        assert!(
            host.query_selector("[name=\"recipientRoot\"]")
                .expect("query")
                .is_none(),
            "no recipient field survives the form",
        );
    }

    /// The half the panel exists for. A guard, not a discovery: it passes
    /// before the form comes out, and its job is to fail if the removal takes
    /// the listing and revocation surface with it.
    #[dialog_common::test]
    fn it_keeps_the_invitation_listing_and_status() {
        let host = injected();
        assert!(
            host.query_selector(".fab__invitation-list")
                .expect("query")
                .is_some(),
            "invitations must still list",
        );
        assert!(
            host.query_selector(".fab__invitation-status")
                .expect("query")
                .is_some(),
            "the status line reports revocation outcomes",
        );
    }
}
